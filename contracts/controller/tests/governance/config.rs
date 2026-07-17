use super::*;
use common::constants::WAD;
use soroban_sdk::testutils::Address as _;

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

// `create_hub` ids start at 1 (the constructor seeds no hub) and each created
// hub is active on return.
#[test]
fn create_hub_assigns_increasing_ids_and_marks_active() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let first = hub::create_hub(&env);
        let second = hub::create_hub(&env);
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert!(storage::get_hub(&env, first).is_some_and(|hub| hub.is_active));
        assert!(storage::get_hub(&env, second).is_some_and(|hub| hub.is_active));
    });
}

// Hub 0 is uncreated and reverts like any inactive hub.
#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_hub_active_rejects_unseeded_hub_zero() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        assert!(storage::get_hub(&env, 0).is_none());
        hub::require_hub_active(&env, 0);
    });
}

#[test]
fn require_hub_active_passes_for_created_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = hub::create_hub(&env);
        hub::require_hub_active(&env, id);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_hub_active_rejects_unknown_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        hub::require_hub_active(&env, 999);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn require_hub_active_rejects_deactivated_hub() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = hub::create_hub(&env);
        storage::set_hub(&env, id, &HubConfig { is_active: false });
        hub::require_hub_active(&env, id);
    });
}

// The setter overrides the defaults stamped at `add_spoke` and the change is
// visible on the next `storage::get_spoke` read.
#[test]
fn set_spoke_liquidation_curve_overrides_defaults() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = spoke::add_spoke(&env);
        let before = storage::get_spoke(&env, id);
        assert_eq!(
            before.liquidation_target_hf_wad,
            crate::constants::DEFAULT_LIQUIDATION_TARGET_HF_WAD
        );
        assert_eq!(
            before.hf_for_max_bonus_wad,
            crate::constants::DEFAULT_HF_FOR_MAX_BONUS_WAD
        );

        spoke::set_spoke_liquidation_curve(
            &env,
            id,
            1_010_000_000_000_000_000,
            995_000_000_000_000_000,
            8_000,
        );

        let after = storage::get_spoke(&env, id);
        assert_eq!(after.liquidation_target_hf_wad, 1_010_000_000_000_000_000);
        assert_eq!(after.hf_for_max_bonus_wad, 995_000_000_000_000_000);
        assert_eq!(after.liquidation_bonus_factor_bps, 8_000);
        // Deprecation flag is untouched by the curve setter.
        assert!(!after.is_deprecated);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #300)")]
fn set_spoke_liquidation_curve_panics_for_unknown_spoke() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        spoke::set_spoke_liquidation_curve(
            &env,
            999,
            1_020_000_000_000_000_000,
            510_000_000_000_000_000,
            10_000,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn set_spoke_liquidation_curve_panics_for_target_hf_at_one() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = spoke::add_spoke(&env);
        spoke::set_spoke_liquidation_curve(
            &env,
            id,
            1_000_000_000_000_000_000,
            500_000_000_000_000_000,
            10_000,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn set_spoke_liquidation_curve_panics_for_hf_for_max_bonus_above_target() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = spoke::add_spoke(&env);
        spoke::set_spoke_liquidation_curve(
            &env,
            id,
            1_020_000_000_000_000_000,
            1_030_000_000_000_000_000,
            10_000,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #134)")]
fn set_spoke_liquidation_curve_panics_for_bonus_factor_above_bps() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        let id = spoke::add_spoke(&env);
        spoke::set_spoke_liquidation_curve(
            &env,
            id,
            1_020_000_000_000_000_000,
            510_000_000_000_000_000,
            10_001,
        );
    });
}

// The owner-gated entrypoints must round-trip through the contract ABI —
// wrapper-level coverage, distinct from the internal-helper tests below.
#[test]
fn min_borrow_floor_entrypoints_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = new_controller(&env);
    let client = crate::ControllerClient::new(&env, &contract);

    let floor = 25 * WAD;
    client.set_min_borrow_collateral_usd(&floor);
    assert_eq!(client.get_min_borrow_collateral_usd(), floor);
}

#[test]
fn blend_pool_approval_entrypoints_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = new_controller(&env);
    let client = crate::ControllerClient::new(&env, &contract);
    let pool = Address::generate(&env);

    assert!(!client.is_blend_pool_approved(&pool));
    client.approve_blend_pool(&pool);
    assert!(client.is_blend_pool_approved(&pool));
    client.revoke_blend_pool(&pool);
    assert!(!client.is_blend_pool_approved(&pool));
}

#[test]
fn token_approval_entrypoints_round_trip() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = new_controller(&env);
    let client = crate::ControllerClient::new(&env, &contract);
    let token = Address::generate(&env);

    client.approve_token(&token);
    env.as_contract(&contract, || {
        assert!(storage::is_token_approved(&env, &token));
    });
    client.revoke_token(&token);
    env.as_contract(&contract, || {
        assert!(!storage::is_token_approved(&env, &token));
    });
}

// `upgrade_pool` must reach the pool lookup: with no pool deployed the
// entrypoint reverts instead of silently returning.
#[test]
fn upgrade_pool_reverts_without_deployed_pool() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = new_controller(&env);
    let client = crate::ControllerClient::new(&env, &contract);

    let bogus = soroban_sdk::BytesN::from_array(&env, &[7u8; 32]);
    assert!(client.try_upgrade_pool(&bogus).is_err());
}

// `remove_delegate` must reach the owner check: a caller that owns no such
// account reverts instead of silently returning.
#[test]
fn remove_delegate_reverts_for_non_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let contract = new_controller(&env);
    let client = crate::ControllerClient::new(&env, &contract);

    let stranger = Address::generate(&env);
    let delegate = Address::generate(&env);
    assert!(client
        .try_remove_delegate(&stranger, &1u64, &delegate)
        .is_err());
}

#[test]
fn min_borrow_floor_defaults_and_blend_wrapper_reflects_storage() {
    let env = Env::default();
    let contract = new_controller(&env);
    env.as_contract(&contract, || {
        // Unset floor returns the default constant (limits.rs 31-33).
        assert_eq!(
            limits::get_min_borrow_collateral_usd(&env),
            crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD
        );
        // Blend-pool wrapper reflects storage both ways (approvals.rs 43-45,48-51).
        let pool = Address::generate(&env);
        assert!(!approvals::is_blend_pool_approved(&env, pool.clone()));
        approvals::set_blend_pool_approval(&env, pool.clone(), true);
        assert!(approvals::is_blend_pool_approved(&env, pool));
    });
}
