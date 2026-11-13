use super::*;
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Env, Event as _};

use crate::test_support::fresh_governance;
use crate::{constants, GovernanceClient};

#[test]
fn constructor_does_not_grant_curator_role() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(
        Governance,
        (admin.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let client = GovernanceClient::new(&env, &contract_id);
    assert!(!client.has_role(&admin, &Symbol::new(&env, CURATOR_ROLE)));
}

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

/// No role restricts execution (audit finding I-02), so `EXECUTOR` is not a
/// governance role: the constructor does not grant it.
#[test]
fn constructor_does_not_grant_an_executor_role() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(
        Governance,
        (admin.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );
    let client = GovernanceClient::new(&env, &contract_id);
    assert!(!client.has_role(&admin, &Symbol::new(&env, "EXECUTOR")));
}

/// A grant or revoke proposal cannot name `EXECUTOR`: it is not a known role.
#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn executor_is_not_a_known_governance_role() {
    let env = Env::default();
    let id = fresh_governance(&env);
    env.as_contract(&id, || {
        require_known_governance_role(&env, &Symbol::new(&env, "EXECUTOR"));
    });
}

#[test]
fn timelocked_grant_names_governance_as_the_caller() {
    let env = Env::default();
    let id = fresh_governance(&env);
    let canceller = Address::generate(&env);
    env.as_contract(&id, || {
        apply_grant_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE));
        assert!(
            access_control::has_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE))
                .is_some()
        );
    });
    // A timelocked grant names governance, not the owner, as the caller.
    let granted = access_control::RoleGranted {
        role: Symbol::new(&env, CANCELLER_ROLE),
        account: canceller,
        caller: id.clone(),
    };
    assert!(env
        .events()
        .all()
        .events()
        .contains(&granted.to_xdr(&env, &id)));
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
        apply_revoke_role(&env, &stranger, &Symbol::new(&env, ORACLE_ROLE), &id);
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
    let granted = access_control::RoleGranted {
        role: Symbol::new(&env, CANCELLER_ROLE),
        account: a,
        caller: id.clone(),
    };
    assert!(env
        .events()
        .all()
        .events()
        .contains(&granted.to_xdr(&env, &id)));
}

/// The constructor emits `ownership_transfer_completed` and
/// `admin_transfer_completed`, so an indexer sees the initial owner and admin.
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

/// A cancel (`live_until_ledger == 0`) removes the mirrored `PendingAdmin` entry
/// without a panic when that entry is already absent.
///
/// `role_transfer::transfer_role` would panic with `NoPendingTransfer` or
/// `InvalidPendingAccount` here. The pending owner and pending admin entries are
/// temporary and archive independently, so such a panic strands the pending
/// owner. The test calls `sync_pending_admin_transfer` directly because the
/// ownable leg of `apply_transfer_ownership` panics first.
#[test]
fn cancelling_the_admin_mirror_tolerates_an_entry_that_is_already_gone() {
    use stellar_access::access_control::AccessControlStorageKey;

    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let stranger = Address::generate(&env);
    let id = env.register(
        Governance,
        (owner.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );

    env.as_contract(&id, || {
        assert!(
            !env.storage()
                .temporary()
                .has(&AccessControlStorageKey::PendingAdmin),
            "nothing pending to begin with"
        );
        // transfer_role would panic NoPendingTransfer here.
        sync_pending_admin_transfer(&env, &stranger, 0);
        assert!(!env
            .storage()
            .temporary()
            .has(&AccessControlStorageKey::PendingAdmin));
    });
}

#[test]
fn cancelling_the_admin_mirror_clears_it_whoever_is_named() {
    use stellar_access::access_control::AccessControlStorageKey;

    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let nominee = Address::generate(&env);
    let stranger = Address::generate(&env);
    let id = env.register(
        Governance,
        (owner.clone(), constants::TIMELOCK_MIN_DELAY_LEDGERS),
    );

    env.as_contract(&id, || {
        sync_pending_admin_transfer(&env, &nominee, 1_000);
        assert!(
            env.storage()
                .temporary()
                .has(&AccessControlStorageKey::PendingAdmin),
            "the mirror must be armed first"
        );

        // A different address than the pending one: transfer_role would panic
        // InvalidPendingAccount, the explicit arm clears regardless.
        sync_pending_admin_transfer(&env, &stranger, 0);
        assert!(
            !env.storage()
                .temporary()
                .has(&AccessControlStorageKey::PendingAdmin),
            "cancelling must clear the mirror"
        );
    });
}
