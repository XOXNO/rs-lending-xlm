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
#[should_panic(expected = "Error(Contract, #45)")]
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

// An unregistered manager reads as absent; activation registers the entry and
// deactivation removes it (absence == inactive), freeing a registry slot.
#[test]
fn position_manager_absent_then_registered_then_removed() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let manager = Address::generate(&env);
        assert!(get_position_manager(&env, &manager).is_none());

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: true });
        assert!(get_position_manager(&env, &manager).is_some_and(|c| c.is_active));
        assert_eq!(position_manager_count(&env), 1);

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        assert!(get_position_manager(&env, &manager).is_none());
        assert_eq!(position_manager_count(&env), 0);

        // Deactivating an unregistered manager cannot underflow the counter.
        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        assert_eq!(position_manager_count(&env), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #45)")]
fn test_position_manager_cap_rejects_overflowing_registration() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        for _ in 0..MAX_POSITION_MANAGERS {
            set_position_manager(
                &env,
                &Address::generate(&env),
                &PositionManagerConfig { is_active: true },
            );
        }
        set_position_manager(
            &env,
            &Address::generate(&env),
            &PositionManagerConfig { is_active: true },
        );
    });
}

// ===== coverage gap-closure tests =====
// blend_pool_allowlist_counter_and_removal (+8) contracts/controller/src/storage/instance.rs:106-126 (uncovered 115,118,119,120,121,122,123,124) + is_blend_pool_approved 84-89
#[test]
fn blend_pool_allowlist_counter_tracks_outstanding_set() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let pool = Address::generate(&env);
        set_blend_pool_approved(&env, &pool, true);
        set_blend_pool_approved(&env, &pool, true); // idempotent re-approve
        assert_eq!(approved_blend_pool_count(&env), 1);
        assert!(is_blend_pool_approved(&env, &pool));

        set_blend_pool_approved(&env, &pool, false);
        assert_eq!(approved_blend_pool_count(&env), 0);
        assert!(!is_blend_pool_approved(&env, &pool));

        // Revoking an unapproved pool cannot underflow the counter.
        set_blend_pool_approved(&env, &pool, false);
        assert_eq!(approved_blend_pool_count(&env), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #45)")]
fn blend_pool_cap_rejects_overflowing_approval() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        for _ in 0..MAX_APPROVED_BLEND_POOLS {
            set_blend_pool_approved(&env, &Address::generate(&env), true);
        }
        set_blend_pool_approved(&env, &Address::generate(&env), true);
    });
}
