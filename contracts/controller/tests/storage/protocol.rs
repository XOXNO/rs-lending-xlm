use super::*;
use crate::Controller;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

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

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        assert!(get_position_manager(&env, &manager).is_none());

        set_position_manager(&env, &manager, &PositionManagerConfig { is_active: false });
        assert!(get_position_manager(&env, &manager).is_none());
    });
}

#[test]
fn blend_pool_allowlist_approve_then_revoke() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));
    env.as_contract(&contract_id, || {
        let pool = Address::generate(&env);
        set_blend_pool_approved(&env, &pool, true);
        set_blend_pool_approved(&env, &pool, true);
        assert!(is_blend_pool_approved(&env, &pool));

        set_blend_pool_approved(&env, &pool, false);
        assert!(!is_blend_pool_approved(&env, &pool));

        set_blend_pool_approved(&env, &pool, false);
        assert!(!is_blend_pool_approved(&env, &pool));
    });
}

#[test]
fn renew_controller_instance_re_extends_instance_ttl() {
    use crate::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
    use soroban_sdk::testutils::storage::Instance as _;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(Controller, (admin,));

    env.as_contract(&contract_id, || {
        crate::storage::renew_controller_instance(&env);
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });

    let aged = TTL_BUMP_INSTANCE - TTL_THRESHOLD_INSTANCE + 1;
    env.ledger().with_mut(|l| l.sequence_number += aged);

    env.as_contract(&contract_id, || {
        assert!(env.storage().instance().get_ttl() < TTL_THRESHOLD_INSTANCE);
        crate::storage::renew_controller_instance(&env);
        assert_eq!(env.storage().instance().get_ttl(), TTL_BUMP_INSTANCE);
    });
}
