use soroban_sdk::{contracterror, contractimpl, contracttype, Env, Vec};

#[contracttype]
#[derive(Clone)]
pub struct Milestone {
    pub verified: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ContractError {
    TooManyMilestones = 1,
}

/// Upper bound for `create_vault` milestone count to keep per-call loops bounded.
pub const MAX_MILESTONES: u32 = 32;

pub struct AccountabilityVaultContract;

#[contractimpl]
impl AccountabilityVaultContract {
    pub fn create_vault(_env: Env, milestones: Vec<Milestone>) -> Result<(), ContractError> {
        if milestones.len() > MAX_MILESTONES {
            return Err(ContractError::TooManyMilestones);
        }

        Ok(())
    }

    pub fn all_verified(_env: Env, milestones: Vec<Milestone>) -> bool {
        let mut i = 0;
        while i < milestones.len() {
            if !milestones.get(i).unwrap().verified {
                return false;
            }
            i += 1;
#![no_std]
//! Disciplr Accountability Vault
//!
//! A Soroban smart contract implementing programmable time-locked capital vaults
//! for accountability staking. A creator stakes funds toward a goal with one or
//! more milestones. A designated verifier set confirms check-ins / milestone
//! completion via M-of-N threshold approval. On success the staked capital is
//! released to the `success_destination`; on a missed deadline the capital is
//! slashed to the `failure_destination` (e.g. a charity or forfeit address).
//!
//! Lifecycle: create_vault -> stake -> (check_in)* -> claim | slash_on_miss
//! Funds movement is modeled via the SEP-41 token client (`stake`, `claim`,
//! `slash_on_miss`, `withdraw`). The contract enforces the state machine,
//! authorization, and deadline rules on-chain.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, BytesN,
    Env, String, Symbol, Vec,
};

/// Maximum allowed horizon between vault creation and its deadline.
///
/// 5 years in seconds. Prevents vaults from locking storage TTL guarantees
/// indefinitely and keeps analytics meaningful.
const MAX_DEADLINE_HORIZON: u64 = 5 * 365 * 24 * 60 * 60;

/// Storage keys for the contract.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// The vault configuration and current state.
    Vault(String),
    /// Per-milestone check-in timestamp (set when the milestone reaches the approval threshold).
    CheckIn(u32),
    /// Per-milestone list of addresses that have approved, used for M-of-N tracking.
    MilestoneApprovals(u32),
    DisputeWindow,
}

/// Lifecycle state of the vault, mirroring the backend `PersistedVault.status`.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultStatus {
    /// Created but not yet funded.
    Draft = 0,
    /// Funded and counting down to its deadline.
    Active = 1,
    /// All milestones verified; funds released to success destination.
    Completed = 2,
    /// Deadline passed without completion; funds slashed.
    Failed = 3,
    /// Cancelled by the creator before activation.
    Cancelled = 4,
    /// Admin hold: entered from Active by the admin (guardian). Blocks
    /// `slash_on_miss` and `claim` until the admin resolves the dispute
    /// back to `Active`, or directly to `Completed` or `Failed`.
    Disputed = 5,
}

/// Verifier configuration for M-of-N milestone approval.
///
/// Grouping these two fields into a single parameter keeps `create_vault`
/// within Soroban's 10-parameter limit while preserving readable call sites.
#[contracttype]
#[derive(Clone)]
pub struct VerifierSet {
    /// Set of addresses authorized to approve milestones via `check_in`.
    pub verifiers: Vec<Address>,
    /// Minimum number of distinct approvals required to verify a milestone.
    pub threshold: u32,
}

/// A single accountability milestone within a vault.
#[contracttype]
#[derive(Clone)]
pub struct Milestone {
    /// Human-readable title describing the milestone goal.
    pub title: String,
    /// Portion of the staked amount tied to this milestone.
    pub amount: i128,
    /// UNIX timestamp (seconds) by which the milestone must be checked in.
    pub due_date: u64,
    /// Whether the verifier has confirmed this milestone.
    pub verified: bool,
    /// Whether this milestone's funds have already been released via `claim_milestone`.
    pub released: bool,
}

/// Full on-chain vault record.
#[contracttype]
#[derive(Clone)]
pub struct Vault {
    /// Address that created the vault and owns the staked funds.
    pub creator: Address,
    /// The party authorized to confirm check-ins / milestones.
    pub verifier: Address,
    /// SEP-41 token used for staking.
    pub token: Address,
    /// Total staked amount (sum of milestone amounts).
    pub amount: i128,
    /// Amount actually transferred into the contract via `stake`.
    pub staked: i128,
    /// Destination for released funds on success.
    pub success_destination: Address,
    /// Destination for slashed funds on a missed deadline.
    pub failure_destination: Address,
    /// Overall vault deadline (seconds since epoch, UTC).
    pub end_timestamp: u64,
    /// Current lifecycle state of the vault.
    pub status: VaultStatus,
    /// Ordered list of milestones with amounts, due dates, and verification status.
    pub milestones: Vec<Milestone>,
    /// Address authorized to pause and unpause this vault in emergencies.
    pub guardian: Address,
    /// When true, `slash_on_miss`, `claim`, and active `withdraw` are blocked.
    pub paused: bool,
}

/// Errors surfaced to callers. Numbered for stable client mapping.
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Error {
    /// Vault storage already exists for the given `vault_id`.
    AlreadyInitialized = 1,
    /// No vault found for the given `vault_id`.
    NotInitialized = 2,
    /// Amount is zero or negative.
    InvalidAmount = 3,
    /// Deadline is in the past, exceeds vault end, or beyond the 5-year horizon.
    InvalidDeadline = 4,
    /// Milestone list is empty.
    NoMilestones = 5,
    /// Operation requires the vault to be in `Draft` state.
    NotDraft = 6,
    /// Operation requires the vault to be in `Active` state.
    NotActive = 7,
    /// Caller is not permitted for this operation (backward compatibility).
    Unauthorized = 8, // backward compatibility
    /// Caller is not the vault creator.
    NotCreator = 23,
    /// Caller is not a member of the verifier set.
    NotVerifier = 24,
    /// Caller is neither the creator nor a verifier.
    NotCreatorOrVerifier = 25,
    /// Vault has already been funded; cannot stake again.
    AlreadyStaked = 9,
    /// Milestone index is outside the valid range.
    MilestoneIndexOutOfRange = 10,
    /// Milestone has already reached the verification threshold.
    MilestoneAlreadyVerified = 11,
    /// Current time is past the milestone or vault deadline.
    DeadlinePassed = 12,
    /// Current time has not yet reached the vault deadline.
    DeadlineNotReached = 13,
    /// Not all milestones have been verified.
    MilestonesIncomplete = 14,
    /// Vault staked balance is zero; nothing to withdraw.
    NothingToWithdraw = 15,
    /// Received amount does not match the declared vault amount.
    AmountMismatch = 16,
}

/// Accountability vault contract entry point.
///
/// Hosts multiple independent vaults keyed by `vault_id`, each enforcing a
/// time-locked staking lifecycle with milestone verification and slash-on-miss.
#[contract]
pub struct AccountabilityVault;

#[contractimpl]
impl AccountabilityVault {
    /// Creates a new accountability vault in `Draft` state.
    ///
    /// Validates that the staked amount is positive, the deadline is in the
    /// future, milestone amounts sum to `amount`, and that there is at least one
    /// milestone. The creator must authorize the call.
    pub fn create_vault(
        env: Env,
        vault_id: String,
        creator: Address,
        verifier: Address,
        token: Address,
        amount: i128,
        success_destination: Address,
        failure_destination: Address,
        end_timestamp: u64,
        milestones: Vec<Milestone>,
        guardian: Address,
    ) -> Result<(), Error> {
        creator.require_auth();

        let key = DataKey::Vault(vault_id);
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyInitialized);
        }
        if verifier_set.verifiers.is_empty() {
            return Err(Error::NoVerifiers);
        }
        if verifier_set.threshold == 0 || verifier_set.threshold > verifier_set.verifiers.len() {
            return Err(Error::InvalidThreshold);
        }
        let verifiers = verifier_set.verifiers;
        let approval_threshold = verifier_set.threshold;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if end_timestamp <= env.ledger().timestamp() {
            return Err(Error::InvalidDeadline);
        }
        if end_timestamp > env.ledger().timestamp() + MAX_DEADLINE_HORIZON {
            return Err(Error::InvalidDeadline);
        }
        if milestones.is_empty() {
            return Err(Error::NoMilestones);
        }
        if failure_destination == creator {
            return Err(Error::InvalidFailureDestination);
        }

        let mut sum: i128 = 0;
        let mut prev_due_date: Option<u64> = None;
        for m in milestones.iter() {
            if m.amount <= 0 || m.amount > MAX_AMOUNT_PER_MILESTONE {
                return Err(Error::InvalidAmount);
            }
            if m.due_date > end_timestamp {
                return Err(Error::InvalidDeadline);
            }
            if let Some(prev) = prev_due_date {
                if m.due_date <= prev {
                    return Err(Error::InvalidDeadline);
                }
            }
            prev_due_date = Some(m.due_date);
            sum += m.amount;
        }
        if sum != amount {
            return Err(Error::AmountMismatch);
        }

        // Ensure all milestones are initialised with released = false.
        let mut init_milestones = Vec::new(&env);
        for m in milestones.iter() {
            init_milestones.push_back(Milestone {
                title: m.title,
                amount: m.amount,
                due_date: m.due_date,
                verified: m.verified,
                released: false,
            });
        }

        let vault = Vault {
            creator: creator.clone(),
            verifier,
            token,
            amount,
            verifier,
            oracle,
            token,
            amount,
            staked: 0,
            success_destination,
            failure_destination,
            end_timestamp,
            status: VaultStatus::Draft,
            milestones,
            guardian,
            paused: false,
        };
        env.storage().persistent().set(&key, &vault);
        Self::extend_ttl(&env, &key);

        env.events()
            .publish((Symbol::new(&env, "vault_created"), creator), amount);
        Ok(())
    }

    /// Funds the vault by transferring `amount` of the staking token from the
    /// creator into the contract, moving the vault from `Draft` to `Active`.
    pub fn stake(env: Env, from: Address) -> Result<(), Error> {
        from.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if vault.status != VaultStatus::Draft {
            return Err(Error::NotDraft);
        }
        if from != vault.creator {
            return Err(Error::Unauthorized);
        }
        if vault.staked != 0 {
            return Err(Error::AlreadyStaked);
        }

        let client = token::Client::new(&env, &vault.token);
        client.transfer(&from, &env.current_contract_address(), &vault.amount);

        vault.staked = vault.amount;
        vault.status = VaultStatus::Active;
        let key = DataKey::Vault(vault_id);
        env.storage().persistent().set(&key, &vault);
        Self::extend_ttl(&env, &key);

        env.events()
            .publish((String::from_str(&env, "vault_staked"), from), vault.amount);
        Ok(())
    }

    /// Records a verifier check-in confirming a milestone before its due date.
    /// Only the designated verifier may call this on an `Active` vault.
    pub fn check_in(env: Env, verifier: Address, milestone_index: u32) -> Result<(), Error> {
        verifier.require_auth();
        let mut vault: Vault = Self::load(&env)?;

        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }
        if verifier != vault.verifier {
            return Err(Error::Unauthorized);
        }
        if milestone_index >= vault.milestones.len() {
            return Err(Error::MilestoneIndexOutOfRange);
        }

        let mut milestone = vault
            .milestones
            .get(milestone_index)
            .ok_or(Error::MilestoneIndexOutOfRange)?;
        if milestone.verified {
            return Err(Error::MilestoneAlreadyVerified);
        }
        if env.ledger().timestamp() > milestone.due_date {
            return Err(Error::DeadlinePassed);
        }

        // M-of-N approval tracking: load or initialize the per-milestone approval list.
        let approvals_key = DataKey::MilestoneApprovals(milestone_index);
        let mut approvals: Vec<Address> = env
            .storage()
            .instance()
            .set(&DataKey::CheckIn(milestone_index), &env.ledger().timestamp());
        env.storage().instance().set(&DataKey::Vault, &vault);
        env.events().publish(
            (String::from_str(&env, "milestone_checked_in"), verifier),
            milestone_index,
        );
        Ok(())
    }

    /// Slashes the staked capital to the `failure_destination` once the vault
    /// deadline has passed and not all milestones were verified. Permissionless:
    /// anyone may trigger the slash after the deadline (e.g. a backend keeper).
    ///
    /// Checks-Effects-Interactions: vault status is set to `Failed` and `staked`
    /// is zeroed in storage BEFORE the external token transfer is executed,
    /// ensuring the terminal state is committed even if the transfer call panics.
    pub fn slash_on_miss(env: Env, vault_id: String) -> Result<(), Error> {
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        // Check Disputed before NotActive so callers get the specific error code.
        if vault.status == VaultStatus::Disputed {
            return Err(Error::VaultDisputed);
        }
        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }
        if vault.paused {
            return Err(Error::Paused);
        }
        if env.ledger().timestamp() <= vault.end_timestamp {
            return Err(Error::DeadlineNotReached);
        }
        if Self::all_verified(&vault) {
            return Err(Error::MilestonesIncomplete);
        }

        // CEI: capture transfer values, update and persist state, then call external token.
        let slashed = vault.staked;
        let failure_destination = vault.failure_destination.clone();
        let token_addr = vault.token.clone();
        vault.status = VaultStatus::Failed;
        vault.staked = 0;
        env.storage().instance().set(&DataKey::Vault(vault_id), &vault);

        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &failure_destination,
            &slashed,
        );

        env.events().publish(
            (
                Symbol::new(&env, "vault_slashed"),
                failure_destination,
            ),
            slashed,
        );
        Ok(())
    }

    /// Releases the staked capital to the `success_destination` once every
    /// milestone has been verified. Callable by the creator or any member of
    /// the verifier set.
    ///
    /// Checks-Effects-Interactions: vault status is set to `Completed` and
    /// `staked` is zeroed in storage BEFORE the external token transfer,
    /// ensuring the terminal state is committed even if the transfer call panics.
    pub fn claim(env: Env, vault_id: String, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        // Check Disputed before NotActive so callers get the specific error code.
        if vault.status == VaultStatus::Disputed {
            return Err(Error::VaultDisputed);
        }
        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }
        if vault.paused {
            return Err(Error::Paused);
        }
        let is_authorized =
            caller == vault.creator || vault.verifiers.iter().any(|v| v == caller);
        if !is_authorized {
            return Err(Error::Unauthorized);
        }
        if !Self::all_verified(&vault) {
            return Err(Error::MilestonesIncomplete);
        }
        // Guard: if any milestone was already claimed via claim_milestone, reject
        // bulk claim to prevent double-release confusion.
        if Self::any_released(&vault) {
            return Err(Error::PartiallyReleased);
        }

        // CEI: capture transfer values, update and persist state, then call external token.
        let released = vault.staked;
        let success_destination = vault.success_destination.clone();
        let token_addr = vault.token.clone();
        vault.status = VaultStatus::Completed;
        vault.staked = 0;
        env.storage().instance().set(&DataKey::Vault(vault_id), &vault);

        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &success_destination,
            &released,
        );

        // Emit vault_completed with both creator (for user attribution) and
        // success_destination (for fund-flow tracking) so analytics can attribute
        // the completion unambiguously without a separate storage lookup.
        env.events().publish(
            (
                Symbol::new(&env, "vault_completed"),
                success_destination,
            ),
            released,
        );
        Ok(())
    }

    /// Releases a single verified milestone's amount to the `success_destination`.
    ///
    /// Callable by the creator or verifier once the milestone at `index` is
    /// verified (via `check_in`). Tracks released milestones on the `Milestone`
    /// struct's `released` flag to prevent double-claiming.
    ///
    /// When the last milestone is claimed, the vault automatically transitions
    /// to `Completed`.
    pub fn claim_milestone(
        env: Env,
        vault_id: String,
        caller: Address,
        index: u32,
    ) -> Result<(), Error> {
        caller.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }
        let is_authorized = caller == vault.creator || vault.verifiers.iter().any(|v| v == caller);
        if !is_authorized {
            return Err(Error::Unauthorized);
        }
        if index >= vault.milestones.len() {
            return Err(Error::MilestoneIndexOutOfRange);
        }

        let mut milestone = vault
            .milestones
            .get(index)
            .ok_or(Error::MilestoneIndexOutOfRange)?;
        if !milestone.verified {
            return Err(Error::MilestonesIncomplete);
        }
        if milestone.released {
            return Err(Error::MilestoneAlreadyReleased);
        }

        let payout = milestone.amount;

        // Mark the milestone as released and update vault.
        milestone.released = true;
        vault.milestones.set(index, milestone);
        vault.staked -= payout;

        let client = token::Client::new(&env, &vault.token);
        client.transfer(
            &env.current_contract_address(),
            &vault.success_destination,
            &payout,
        );

        env.events().publish(
            (
                Symbol::new(&env, "milestone_claimed"),
                vault.success_destination.clone(),
            ),
            (index, payout),
        );

        // Transition to Completed if every milestone has now been released.
        if Self::all_released(&vault) {
            vault.status = VaultStatus::Completed;
            // Emit vault_completed with both creator (for user attribution) and
            // success_destination (for fund-flow tracking) so analytics can attribute
            // the completion unambiguously without a separate storage lookup.
            env.events().publish(
                (
                    Symbol::new(&env, "vault_completed"),
                    vault.success_destination.clone(),
                ),
                vault.amount,
            );
        }

        env.storage().instance().set(&DataKey::Vault(vault_id), &vault);
        Ok(())
    }

    /// Cancels an unfunded (`Draft`) vault. Only the creator may cancel a
    /// draft; this path does not transfer tokens and emits `vault_cancelled`.
    pub fn cancel_vault(env: Env, vault_id: String, creator: Address) -> Result<(), Error> {
        creator.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if creator != vault.creator {
            return Err(Error::Unauthorized);
        }
        if vault.status == VaultStatus::Draft {
            vault.status = VaultStatus::Cancelled;
            let key = DataKey::Vault(vault_id);
            env.storage().persistent().set(&key, &vault);
            Self::extend_ttl(&env, &key);

            env.events()
                .publish((Symbol::new(&env, "vault_cancelled"), creator), 0i128);
            return Ok(());
        }

        vault.status = VaultStatus::Cancelled;
        let key = DataKey::Vault(vault_id);
        env.storage().persistent().set(&key, &vault);
        Self::extend_ttl(&env, &key);

        env.events()
            .publish((String::from_str(&env, "vault_cancelled"), creator), 0i128);
        Ok(())
    }

    /// Refunds the creator for an `Active` vault that was never checked-in.
    /// This function is restricted to `Active` refund cases; callers that wish
    /// to cancel a Draft should call `cancel_vault` instead.
    pub fn withdraw(env: Env, vault_id: String, creator: Address) -> Result<(), Error> {
        creator.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if creator != vault.creator {
            return Err(Error::Unauthorized);
        }
        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }
        if vault.paused {
            return Err(Error::Paused);
        }
        if Self::any_verified(&vault) {
            return Err(Error::Unauthorized);
        }
        if vault.staked == 0 {
            return Err(Error::NothingToWithdraw);
        }

        // CEI: capture transfer values, update and persist state, then call external token.
        let refunded = vault.staked;
        let token_addr = vault.token.clone();
        vault.staked = 0;
        vault.status = VaultStatus::Cancelled;
        env.storage().instance().set(&DataKey::Vault, &vault);

        token::Client::new(&env, &token_addr).transfer(
            &env.current_contract_address(),
            &creator,
            &refunded,
        );

        env.events().publish(
            (Symbol::new(&env, "vault_withdrawn"), creator),
            refunded,
        );
        Ok(())
    }

    /// Transitions an `Active` vault into `Disputed`, blocking `slash_on_miss` and
    /// `claim` until an admin resolves the dispute.
    ///
    /// Only the `guardian` address may call this. The vault must be `Active`.
    pub fn admin_dispute(env: Env, vault_id: String, admin: Address) -> Result<(), Error> {
        admin.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if admin != vault.guardian {
            return Err(Error::Unauthorized);
        }
        if vault.status != VaultStatus::Active {
            return Err(Error::NotActive);
        }

        vault.status = VaultStatus::Disputed;
        let key = DataKey::Vault(vault_id);
        env.storage().persistent().set(&key, &vault);
        Self::extend_ttl(&env, &key);

        env.events()
            .publish((String::from_str(&env, "vault_disputed"), admin), ());
        Ok(())
    }

    /// Resolves a `Disputed` vault to `Active`, `Completed`, or `Failed`.
    ///
    /// Only the `guardian` address may call this. `target` must be one of those
    /// three statuses; any other value is rejected with `Error::NotActive`.
    ///
    /// Resolving to `Completed` or `Failed` is a terminal administrative decision
    /// and does **not** trigger a token transfer — settlement still goes through
    /// `claim` (for Completed) or `slash_on_miss` (for Failed) once the vault is
    /// back in the appropriate resolved state.
    pub fn admin_resolve(
        env: Env,
        vault_id: String,
        admin: Address,
        target: VaultStatus,
    ) -> Result<(), Error> {
        admin.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if admin != vault.guardian {
            return Err(Error::Unauthorized);
        }
        if vault.status != VaultStatus::Disputed {
            return Err(Error::VaultDisputed);
        }

        match target {
            VaultStatus::Active | VaultStatus::Completed | VaultStatus::Failed => {}
            _ => return Err(Error::NotActive),
        }

        vault.status = target;
        let key = DataKey::Vault(vault_id);
        env.storage().persistent().set(&key, &vault);
        Self::extend_ttl(&env, &key);

        env.events()
            .publish((String::from_str(&env, "vault_dispute_resolved"), admin), target as u32);
        Ok(())
    }

    /// Pauses the vault, blocking `slash_on_miss`, `claim`, and active `withdraw`.
    ///
    /// Only the `guardian` address set at vault creation may call this function.
    /// Use to halt settlement during disputes or detected incidents.
    pub fn emergency_pause(
        env: Env,
        vault_id: String,
        guardian: Address,
    ) -> Result<(), Error> {
        guardian.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if guardian != vault.guardian {
            return Err(Error::Unauthorized);
        }
        vault.paused = true;
        env.storage().instance().set(&DataKey::Vault(vault_id), &vault);
        env.events()
            .publish((Symbol::new(&env, "vault_paused"), guardian), true);
        Ok(())
    }

    /// Unpauses the vault, re-enabling `slash_on_miss`, `claim`, and `withdraw`.
    ///
    /// Only the `guardian` address set at vault creation may call this function.
    pub fn emergency_unpause(
        env: Env,
        vault_id: String,
        guardian: Address,
    ) -> Result<(), Error> {
        guardian.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if guardian != vault.guardian {
            return Err(Error::Unauthorized);
        }
        vault.paused = false;
        env.storage().instance().set(&DataKey::Vault(vault_id), &vault);
        env.events()
            .publish((Symbol::new(&env, "vault_unpaused"), guardian), false);
        Ok(())
    }

    /// Read-only accessor returning the current vault record.
    pub fn get_vault(env: Env, vault_id: String) -> Result<Vault, Error> {
        Self::load(&env, &vault_id)
    }

    /// Sweeps any residual token balance held by the contract to the vault creator
    /// after a terminal settlement. Only the creator may call this, and only once
    /// `staked` has been zeroed by `claim`, `slash_on_miss`, or `withdraw`.
    pub fn reclaim_after_settlement(env: Env, vault_id: String, token_address: Address) -> Result<(), Error> {
        let vault: Vault = Self::load(&env, &vault_id)?;
        vault.creator.require_auth();

        // Only sweep after the vault has no outstanding stake.
        if vault.staked != 0 {
            return Err(Error::StakedRemaining);
        }

        let contract_addr = env.current_contract_address();
        let client = token::Client::new(&env, &token_address);
        let bal = client.balance(&contract_addr);
        if bal > 0 {
            client.transfer(&contract_addr, &vault.creator, &bal);
        }
        Ok(())
    }

    // ── internal helpers ────────────────────────────────────────────────


    pub fn configure_window(env: Env, window: u64) {
        env.storage().instance().set(&DataKey::DisputeWindow, &window);
    }

    pub fn dispute_milestone(env: Env, vault_id: String, creator: Address, index: u32) -> Result<(), Error> {
        creator.require_auth();
        let mut vault: Vault = Self::load(&env, &vault_id)?;

        if vault.creator != creator {
            return Err(Error::Unauthorized);
        }
        if index >= vault.milestones.len() {
            return Err(Error::MilestoneIndexOutOfRange);
        }

        let mut milestone = vault.milestones.get(index).unwrap();
        if !milestone.verified {
            return Err(Error::MilestonesIncomplete);
        }

        let dispute_window: u64 = env.storage().instance().get(&DataKey::DisputeWindow).unwrap_or(86400);
        let verified_at: u64 = env.storage().instance().get(&DataKey::CheckIn(index)).unwrap_or(0);
        
        if env.ledger().timestamp() > verified_at + dispute_window {
            return Err(Error::DeadlinePassed);
        }

        milestone.verified = false;
        vault.milestones.set(index, milestone);
        
        // Match upstream's storage format
        env.storage().instance().set(&DataKey::Vault, &vault);

        let event_name = soroban_sdk::Symbol::new(&env, "milestone_disputed");
        env.events().publish((event_name, creator), index);
        
        Ok(())
    }
    fn load(env: &Env, vault_id: &String) -> Result<Vault, Error> {
        let key = DataKey::Vault(vault_id.clone());
        let vault = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NotInitialized)?;
        Self::extend_ttl(env, &key);
        Ok(vault)
    }

    fn extend_ttl(env: &Env, key: &DataKey) {
        // Persistent storage TTL bumping: 30 days threshold, 30 days bump.
        // Approx 17280 ledgers per day.
        let threshold = 30 * 17280;
        let bump = 30 * 17280;
        env.storage().persistent().extend_ttl(key, threshold, bump);
    }

    fn all_verified(vault: &Vault) -> bool {
        for m in vault.milestones.iter() {
            if !m.verified {
                return false;
            }
        }
        true
    }

    pub fn any_verified(_env: Env, milestones: Vec<Milestone>) -> bool {
        let mut i = 0;
        while i < milestones.len() {
            if milestones.get(i).unwrap().verified {
                return true;
            }
            i += 1;
    fn any_verified(vault: &Vault) -> bool {
        for m in vault.milestones.iter() {
            if m.verified {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod test;
