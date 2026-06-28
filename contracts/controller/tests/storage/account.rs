use super::*;
use crate::Controller;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// Re-adding the same delegate is a no-op; the list never double-counts.
#[test]
fn add_delegate_is_idempotent() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 7u64;
        let delegate = Address::generate(&env);

        add_delegate(&env, account_id, &delegate);
        add_delegate(&env, account_id, &delegate);

        let delegates = get_delegates(&env, account_id);
        assert_eq!(delegates.len(), 1);
        assert!(delegates.contains(delegate.clone()));
    });
}

// Removing a delegate revokes it and leaves an empty, entry-free list.
#[test]
fn remove_delegate_revokes_access() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 9u64;
        let keep = Address::generate(&env);
        let drop = Address::generate(&env);

        add_delegate(&env, account_id, &keep);
        add_delegate(&env, account_id, &drop);
        remove_delegate(&env, account_id, &drop);

        let delegates = get_delegates(&env, account_id);
        assert_eq!(delegates.len(), 1);
        assert!(delegates.contains(keep));
        assert!(!delegates.contains(drop));
    });
}

// Removing an absent delegate is a no-op and never underflows.
#[test]
fn remove_absent_delegate_is_noop() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 11u64;
        remove_delegate(&env, account_id, &Address::generate(&env));
        assert_eq!(get_delegates(&env, account_id).len(), 0);
    });
}

// The delegate list is bounded; growth past the cap is rejected.
#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn add_delegate_rejects_overflowing_cap() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let account_id = 13u64;
        for _ in 0..MAX_DELEGATES {
            add_delegate(&env, account_id, &Address::generate(&env));
        }
        add_delegate(&env, account_id, &Address::generate(&env));
    });
}
