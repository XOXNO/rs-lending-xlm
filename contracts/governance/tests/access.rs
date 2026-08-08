use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

use crate::test_support::fresh_governance;
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

#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn grant_role_enforces_canceller_executor_separation() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let delegate = Address::generate(&env);
    env.as_contract(&id, || {
        apply_grant_role(&env, &delegate, &Symbol::new(&env, EXECUTOR_ROLE));
        apply_grant_role(&env, &delegate, &Symbol::new(&env, CANCELLER_ROLE));
    });
}

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

#[test]
fn grant_role_allows_owner_to_hold_executor_and_canceller() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let id = env.register(
        Governance,
        (owner.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    env.as_contract(&id, || {
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

#[test]
fn accepting_self_transfer_preserves_all_owner_roles() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let id = env.register(
        Governance,
        (owner.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let client = GovernanceClient::new(&env, &id);

    env.as_contract(&id, || apply_transfer_ownership(&env, &owner, 1_000));
    client.accept_ownership();

    for role in default_operational_roles(&env) {
        assert!(client.has_role(&owner, &role));
    }
}

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

#[test]
fn canceller_reset_grants_each_member_once() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let a = Address::generate(&env);
    env.as_contract(&id, || {
        let role = Symbol::new(&env, CANCELLER_ROLE);
        let owner = owner_or_panic(&env);
        assert_eq!(access_control::get_role_member_count(&env, &role), 1);
        let mut new = soroban_sdk::Vec::new(&env);
        new.push_back(a.clone());
        new.push_back(a.clone());
        apply_canceller_reset(&env, &new);
        assert!(access_control::has_role(&env, &a, &role).is_some());
        assert!(access_control::has_role(&env, &owner, &role).is_some());
        assert_eq!(access_control::get_role_member_count(&env, &role), 2);
    });
}

/// The constructor's owner and admin must be reachable from events alone.
///
/// `ownable::set_owner` and `access_control::set_admin` are silent storage
/// writes, so without these emissions the governance contract's owner and admin
/// are invisible to an event-sourced indexer — the `*_transfer_*` events only
/// fire on a later handover. Roles and the timelock delay were already
/// observable (`grant_role_no_auth` and `set_min_delay` emit).
#[test]
fn constructor_emits_owner_and_admin() {
    use soroban_sdk::testutils::Events as _;
    use soroban_sdk::xdr::{ContractEventBody, ScSymbol, ScVal};

    let env = Env::default();
    let admin = Address::generate(&env);
    env.register(Governance, (admin.clone(), 12u32));

    let symbol = |name: &str| ScVal::Symbol(ScSymbol(name.try_into().unwrap()));
    let mut saw_owner = false;
    let mut saw_admin = false;

    for event in env.events().all().events().iter() {
        let ContractEventBody::V0(body) = &event.body;
        match body.topics.as_slice().first() {
            Some(t) if t == &symbol("ownership_transfer_completed") => saw_owner = true,
            Some(t) if t == &symbol("admin_transfer_completed") => saw_admin = true,
            _ => {}
        }
    }

    assert!(saw_owner, "constructor must publish the initial owner");
    assert!(saw_admin, "constructor must publish the initial admin");
}
