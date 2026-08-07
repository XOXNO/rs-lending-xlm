use super::*;
use common::types::{HubAssetKey, SpokeAssetConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::storage;
use crate::Controller;

fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

fn listing(paused: bool, frozen: bool) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused,
        frozen,
        loan_to_value: 8_000,
        liquidation_threshold: 8_500,
        liquidation_bonus: 500,
        liquidation_fees: 0,
        supply_cap: 1_000_000,
        borrow_cap: 1_000_000,
    }
}

fn seed_listing(env: &Env, spoke_id: u32, asset: &Address, paused: bool, frozen: bool) {
    storage::set_spoke_asset(env, spoke_id, &hub(asset), &listing(paused, frozen));
}

#[test]
fn set_spoke_asset_flags_tightens_pause_and_freeze() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, false, false);

        set_spoke_asset_flags(&env, 1, hub(&asset), true, false);
        let after_pause = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(after_pause.paused);
        assert!(!after_pause.frozen);

        set_spoke_asset_flags(&env, 1, hub(&asset), true, true);
        let after_both = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(after_both.paused);
        assert!(after_both.frozen);
        // Risk params must be untouched by the flags-only path.
        assert_eq!(after_both.loan_to_value, 8_000);
        assert_eq!(after_both.supply_cap, 1_000_000);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn set_spoke_asset_flags_rejects_unpause() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, false);
        set_spoke_asset_flags(&env, 1, hub(&asset), false, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #317)")]
fn set_spoke_asset_flags_rejects_unfreeze() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, true);
        // Keep pause; clear freeze only — still a relaxation.
        set_spoke_asset_flags(&env, 1, hub(&asset), true, false);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn set_spoke_asset_flags_rejects_unknown_listing() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        set_spoke_asset_flags(&env, 1, hub(&asset), true, false);
    });
}

#[test]
fn flag_ratchet_allows_idempotent_tighten() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&contract, || {
        seed_listing(&env, 1, &asset, true, true);
        // Re-asserting the same flags is not a relaxation.
        set_spoke_asset_flags(&env, 1, hub(&asset), true, true);
        let cfg = storage::get_spoke_asset(&env, 1, &hub(&asset)).unwrap();
        assert!(cfg.paused && cfg.frozen);
    });
}
