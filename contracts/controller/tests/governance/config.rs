use super::*;
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
        assert_eq!(before.liquidation_target_hf_wad, 1_020_000_000_000_000_000);

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
        spoke::set_spoke_liquidation_curve(&env, id, 1_000_000_000_000_000_000, 500_000_000_000_000_000, 10_000);
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
