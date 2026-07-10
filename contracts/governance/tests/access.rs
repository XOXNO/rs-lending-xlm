use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

use crate::{constants, GovernanceClient};

#[test]
fn constructor_grants_oracle_role_to_admin() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(
        Governance,
        (admin.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let client = GovernanceClient::new(&env, &contract_id);

    assert!(client.has_role(&admin, &Symbol::new(&env, ORACLE_ROLE)));
    env.as_contract(&contract_id, || {
        assert_eq!(ownable::get_owner(&env), Some(admin.clone()));
        assert_eq!(access_control::get_admin(&env), Some(admin));
    });
}

fn fresh_governance(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Governance, (admin, constants::TIMELOCK_MIN_DELAY_LEDGERS))
}

// Delegates cannot hold both EXECUTOR and CANCELLER.
#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn grant_role_enforces_executor_canceller_separation() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let delegate = Address::generate(&env);
    env.as_contract(&id, || {
        apply_grant_role(&env, &delegate, &Symbol::new(&env, CANCELLER_ROLE));
        apply_grant_role(&env, &delegate, &Symbol::new(&env, EXECUTOR_ROLE));
    });
}

// Separate EXECUTOR and CANCELLER delegates are allowed.
#[test]
fn grant_role_allows_separated_executor_and_canceller() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let executor = Address::generate(&env);
    let canceller = Address::generate(&env);
    env.as_contract(&id, || {
        apply_grant_role(&env, &executor, &Symbol::new(&env, EXECUTOR_ROLE));
        apply_grant_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE));
        assert!(
            access_control::has_role(&env, &executor, &Symbol::new(&env, EXECUTOR_ROLE)).is_some()
        );
        assert!(
            access_control::has_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE))
                .is_some()
        );
    });
}

// The owner is exempt from the EXECUTOR/CANCELLER separation: it holds both
// from construction, and must be able to re-acquire either through the
// timelocked grant path (e.g. to restore a CANCELLER accidentally revoked)
// even while still holding EXECUTOR. Regression for the missing owner check in
// require_executor_canceller_separation, which previously panicked here.
#[test]
fn grant_role_allows_owner_to_hold_executor_and_canceller() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let id = env.register(
        Governance,
        (owner.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    env.as_contract(&id, || {
        // Owner already holds EXECUTOR from the constructor; granting the
        // conflicting CANCELLER (and vice versa) must not panic for the owner.
        apply_grant_role(&env, &owner, &Symbol::new(&env, CANCELLER_ROLE));
        apply_grant_role(&env, &owner, &Symbol::new(&env, EXECUTOR_ROLE));
        assert!(
            access_control::has_role(&env, &owner, &Symbol::new(&env, EXECUTOR_ROLE)).is_some()
        );
        assert!(
            access_control::has_role(&env, &owner, &Symbol::new(&env, CANCELLER_ROLE)).is_some()
        );
    });
}

// Revoke requires the account to hold the role.
#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn revoke_role_rejects_unheld() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let stranger = Address::generate(&env);
    env.as_contract(&id, || {
        apply_revoke_role(&env, &stranger, &Symbol::new(&env, ORACLE_ROLE));
    });
}
