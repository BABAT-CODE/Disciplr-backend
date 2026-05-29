#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, vec, Address, Env, String,
};

fn create_token(env: &Env, admin: &Address) -> (Address, token::StellarAssetClient<'static>) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let address = sac.address();
    (
        address.clone(),
        token::StellarAssetClient::new(env, &address),
    )
}

struct Setup {
    env: Env,
    contract: AccountabilityVaultClient<'static>,
    token: Address,
    creator: Address,
    verifier: Address,
    success: Address,
    failure: Address,
}

fn setup(milestone_due_offsets: &[u64], amounts: &[i128]) -> Setup {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let creator = Address::generate(&env);
    let verifier = Address::generate(&env);
    let success = Address::generate(&env);
    let failure = Address::generate(&env);
    let token_admin = Address::generate(&env);

    let (token, token_admin_client) = create_token(&env, &token_admin);
    let total: i128 = amounts.iter().sum();
    token_admin_client.mint(&creator, &total);

    let contract_id = env.register(AccountabilityVault, ());
    let contract = AccountabilityVaultClient::new(&env, &contract_id);

    let mut milestones = vec![&env];
    for (i, due) in milestone_due_offsets.iter().enumerate() {
        milestones.push_back(Milestone {
            title: String::from_str(&env, "m"),
            amount: amounts[i],
            due_date: 1_000 + due,
            verified: false,
        });
    }

    let end = 1_000 + milestone_due_offsets.iter().max().copied().unwrap_or(0);
    contract.create_vault(
        &creator,
        &verifier,
        &token,
        &total,
        &success,
        &failure,
        &end,
        &milestones,
    );

    let result = Contract::create_vault(
        env,
        contract,
        token,
        creator,
        600, // Total amount matches sum of milestones
        verifier,
        success,
        failure,
    }
}

#[test]
fn test_create_and_stake() {
    let s = setup(&[100], &[500]);
    let vault = s.contract.get_vault();
    assert_eq!(vault.status, VaultStatus::Draft);

    s.contract.stake(&s.creator);
    let vault = s.contract.get_vault();
    assert_eq!(vault.status, VaultStatus::Active);
    assert_eq!(vault.staked, 500);

    let token_client = token::Client::new(&s.env, &s.token);
    assert_eq!(token_client.balance(&s.creator), 0);
}

#[test]
fn test_check_in_and_claim_success() {
    let s = setup(&[100, 200], &[300, 700]);
    s.contract.stake(&s.creator);

    s.contract.check_in(&s.verifier, &0);
    s.contract.check_in(&s.verifier, &1);

    s.contract.claim(&s.creator);
    let vault = s.contract.get_vault();
    assert_eq!(vault.status, VaultStatus::Completed);

    let token_client = token::Client::new(&s.env, &s.token);
    assert_eq!(token_client.balance(&s.success), 1000);
}

#[test]
fn test_slash_on_miss() {
    let s = setup(&[100], &[500]);
    s.contract.stake(&s.creator);

    // Advance past the deadline without any check-in.
    s.env.ledger().set_timestamp(2_000);
    s.contract.slash_on_miss();

    let vault = s.contract.get_vault();
    assert_eq!(vault.status, VaultStatus::Failed);

    let token_client = token::Client::new(&s.env, &s.token);
    assert_eq!(token_client.balance(&s.failure), 500);
}

#[test]
fn test_withdraw_draft_cancels() {
    let s = setup(&[100], &[500]);
    s.contract.withdraw(&s.creator);
    let vault = s.contract.get_vault();
    assert_eq!(vault.status, VaultStatus::Cancelled);
}

// ── cross-feature: stake_from then oracle check_in then claim ────────────────

#[test]
#[should_panic]
fn test_claim_before_all_verified_fails() {
    let s = setup(&[100, 200], &[300, 700]);
    s.contract.stake(&s.creator);
    s.contract.check_in(&s.verifier, &0);
    // Second milestone not yet verified -> claim must fail.
    s.contract.claim(&s.creator);
}

#[test]
#[should_panic]
fn test_slash_before_deadline_fails() {
    let s = setup(&[100], &[500]);
    s.contract.stake(&s.creator);
    s.contract.slash_on_miss();
}



