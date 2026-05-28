# Contracts

This folder contains on-chain contract implementations used by the project.

## Accountability Vault: precedence rule

- `withdraw` is permitted only when the current time is less than or equal to the vault `deadline`.
- `slash_on_miss` is permitted only when the current time is strictly greater than the vault `deadline`.
- A single-spend guard prevents the `staked` funds from being paid out more than once. Only one of `withdraw` or `slash_on_miss` may succeed; they are mutually exclusive.

The `contracts/accountability_vault` crate includes unit tests that exercise the race between `withdraw` and `slash_on_miss`.
