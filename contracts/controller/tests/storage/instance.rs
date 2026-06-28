use super::*;
use crate::Controller;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

// Approve/revoke/consume keeps the outstanding counter exact: re-approval
// of the same token cannot double-count, and revocation frees a slot.
#[test]
fn test_token_approval_counter_tracks_outstanding_set() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let token = Address::generate(&env);
        set_token_approved(&env, &token, true);
        set_token_approved(&env, &token, true); // idempotent re-approve
        assert_eq!(approved_token_count(&env), 1);
        assert!(is_token_approved(&env, &token));

        set_token_approved(&env, &token, false);
        assert_eq!(approved_token_count(&env), 0);
        assert!(!is_token_approved(&env, &token));

        // Revoking an unapproved token cannot underflow the counter.
        set_token_approved(&env, &token, false);
        assert_eq!(approved_token_count(&env), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn test_token_approval_cap_rejects_overflowing_approval() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        for _ in 0..MAX_OUTSTANDING_TOKEN_APPROVALS {
            set_token_approved(&env, &Address::generate(&env), true);
        }
        set_token_approved(&env, &Address::generate(&env), true);
    });
}

// An unregistered manager reads as absent; a write round-trips the active flag.
#[test]
fn position_manager_absent_then_active_flag_round_trips() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let manager = Address::generate(&env);
        assert!(get_position_manager(&env, &manager).is_none());

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        assert!(get_position_manager(&env, &manager).is_some_and(|c| c.is_active));

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        assert!(get_position_manager(&env, &manager).is_some_and(|c| !c.is_active));
    });
}
