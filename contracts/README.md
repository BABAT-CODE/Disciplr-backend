# Disciplr Soroban Contracts

On-chain programmable, time-locked capital vaults for accountability staking,
the chain-side counterpart to the `disciplr-backend` API and Horizon listener.

## Workspace layout

```text
contracts/
├── Cargo.toml                       # workspace manifest (soroban-sdk = "23")
├── README.md
└── accountability_vault/
    ├── Cargo.toml
    └── src/
        ├── lib.rs                   # AccountabilityVault contract
        └── test.rs                  # unit tests (testutils)
```

## accountability_vault

Implements the vault lifecycle that the backend models off-chain in
`src/services/vaultTransitions.ts` and parses events for in
`src/services/eventParser.ts`:

| Function | Purpose |
|---|---|
| `create_vault` | Create a `Draft` vault with milestones, verifier, optional oracle, and success/failure destinations. Validates amount, deadline, and that milestone amounts sum to the total. |
| `stake` | Creator transfers the SEP-41 token into the contract; `Draft` -> `Active`. Staked amount is measured as the actual contract balance delta to handle fee-on-transfer tokens. |
| `stake_from` | Allowance-based staking variant using SEP-41 `transfer_from`. Enables a backend/spender account to drive staking after the creator calls `token.approve(spender, amount)`. Same balance-delta safety check as `stake`. |
| `check_in` | Designated verifier **or** the optional oracle address confirms a milestone before its `due_date`. The emitted event includes a `source` topic (`"verifier"` or `"oracle"`) for backend parser disambiguation. |
| `extend_deadline` | Jointly extend `end_timestamp` — requires `require_auth` from both `creator` and `verifier`. Only permitted while the vault is `Active` and before the current deadline. All existing milestone `due_date` values must remain `<= new_end_timestamp`. |
| `slash_on_miss` | After the deadline with unverified milestones, slash funds to `failure_destination`; `Active` -> `Failed`. |
| `claim` | When all milestones are verified, release funds to `success_destination`; `Active` -> `Completed`. |
| `withdraw` | Cancel/refund an unfunded or unstarted vault to the creator; -> `Cancelled`. |
| `get_vault` | Read-only accessor for the current vault record. |

### Oracle role

Pass an `oracle: Option<Address>` to `create_vault`. When set, the oracle
address may call `check_in` in addition to the designated `verifier`. This
enables the backend oracle job (`src/jobs/handlers.ts`) to automate milestone
confirmation on-chain without requiring a human signer.

The `milestone_checked_in` event carries a `source` topic (`"verifier"` or
`"oracle"`) so `src/services/eventParser.ts` can distinguish automated
confirmations from manual ones.

### Allowance-based staking (`stake_from`)

`stake_from(from, spender)` pulls tokens via `transfer_from`, letting an
authorized backend or smart-contract account drive the staking flow once the
creator has called `token.approve(spender, amount)`. The function verifies the
spender's allowance before attempting the transfer and sets `vault.staked` to
the measured balance delta, consistent with the `stake` behavior.

### Deadline extension rules

`extend_deadline(creator, verifier, new_end_timestamp)`:

- Both `creator` and `verifier` must sign (dual `require_auth`).
- The vault must be `Active`; terminal states (`Completed`, `Failed`,
  `Cancelled`) are immutable.
- The current ledger time must be **before** the existing `end_timestamp`
  (no retroactive extensions).
- `new_end_timestamp` must be strictly **greater** than the current value.
- Every milestone `due_date` must be `<= new_end_timestamp` after the
  extension; milestones cannot silently outlive the vault deadline.

### Balance delta assertion

Both `stake` and `stake_from` measure the actual token amount received by
reading the contract's token balance before and after the transfer. If the
received amount is less than `vault.amount` (e.g. a fee-on-transfer token
deducted a fee), the call is rejected with `Error::AmountMismatch` and
`vault.staked` is never written. This prevents the contract from settling
claims or slashes against a balance it never actually held.

### Error codes

| Code | Name | Meaning |
|---|---|---|
| 1 | `AlreadyInitialized` | `create_vault` called on an already-initialized contract |
| 2 | `NotInitialized` | Vault not yet created |
| 3 | `InvalidAmount` | Non-positive total or milestone amount |
| 4 | `InvalidDeadline` | Deadline in the past, milestone due_date > end_timestamp, or extension is not strictly greater |
| 5 | `NoMilestones` | Empty milestone list |
| 6 | `NotDraft` | `stake`/`stake_from` called on a non-Draft vault |
| 7 | `NotActive` | Operation requires an Active vault |
| 8 | `Unauthorized` | Caller is not the required party (creator / verifier / oracle) |
| 9 | `AlreadyStaked` | `stake`/`stake_from` called when `staked != 0` |
| 10 | `MilestoneIndexOutOfRange` | Milestone index >= milestones length |
| 11 | `MilestoneAlreadyVerified` | `check_in` on an already-verified milestone |
| 12 | `DeadlinePassed` | `check_in` after milestone `due_date`, or `extend_deadline` after vault `end_timestamp` |
| 13 | `DeadlineNotReached` | `slash_on_miss` called before the deadline |
| 14 | `MilestonesIncomplete` | `claim` with unverified milestones, or `slash_on_miss` when all verified |
| 15 | `NothingToWithdraw` | `withdraw` with zero staked balance |
| 16 | `AmountMismatch` | Milestone amounts don't sum to total, or balance delta < declared amount |
| 17 | `InsufficientAllowance` | `stake_from` spender allowance < vault amount |

The `VaultStatus` enum (`Draft`/`Active`/`Completed`/`Failed`/`Cancelled`)
mirrors `PersistedVault.status` in `src/types/vaults.ts`. Emitted events
(`vault_created`, `vault_staked`, `milestone_checked_in`, `deadline_extended`,
`vault_slashed`, `vault_completed`, `vault_cancelled`, `vault_withdrawn`) align
with the topics consumed by the backend event parser.

## Build & test

```bash
# from the contracts/ directory
stellar contract build
cargo test
```

## Backend integration

`src/services/soroban.ts` calls `create_vault` via the Stellar SDK
(`@stellar/stellar-sdk` v14). The Horizon listener
(`src/services/horizonListener.ts`) and `src/services/eventParser.ts`
ingest the events emitted by these functions to keep the off-chain vault state
in sync.
