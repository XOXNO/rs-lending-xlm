use super::*;
use crate::storage;
use crate::Controller;
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{
    Account, AssetConfig, MarketIndexRaw, PositionMode, SpokeAssetConfig, SpokeUsageRaw,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Map};

fn spoke_asset_config(ltv_bps: u32) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: ltv_bps,
        liquidation_threshold: ltv_bps + 500,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 0,
        borrow_cap: 0,
    }
}

fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

fn new_controller(env: &Env) -> Address {
    let admin = Address::generate(env);
    env.register(Controller, (admin,))
}

// A spoke listing converts to AssetConfig with the listed LTV and flags.
#[test]
fn require_spoke_asset_converts_listed_risk_config() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Cache::new_view(&env);
        let cfg: AssetConfig = (&cache.require_spoke_asset(1, &hub(&asset))).into();
        assert_eq!(cfg.loan_to_value.raw() as u32, 9_000);
        assert!(cfg.can_supply());
        assert!(cfg.can_borrow());
    });
}

// Each spoke resolves risk from its own SpokeAsset(spoke_id) listing; one spoke's
// config does not bleed into another's.
#[test]
fn require_spoke_asset_reads_each_spoke_directly() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        storage::set_spoke_asset(&env, 2, &hub(&asset), &spoke_asset_config(5_000));
        // One cache binds to one spoke per transaction, so each spoke resolves
        // through its own cache (mirroring one account = one spoke per tx).
        let mut cache_spoke_1 = Cache::new_view(&env);
        let cfg1: AssetConfig = (&cache_spoke_1.require_spoke_asset(1, &hub(&asset))).into();
        assert_eq!(cfg1.loan_to_value.raw() as u32, 9_000);
        let mut cache_spoke_2 = Cache::new_view(&env);
        let cfg2: AssetConfig = (&cache_spoke_2.require_spoke_asset(2, &hub(&asset))).into();
        assert_eq!(cfg2.loan_to_value.raw() as u32, 5_000);
    });
}

// An asset not listed on the account's spoke is rejected (#307 AssetNotInSpoke).
#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn require_spoke_asset_panics_when_unlisted_on_spoke() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Cache::new_view(&env);
        let _: SpokeAssetConfig = cache.require_spoke_asset(2, &hub(&asset));
    });
}

// Grandfathering: lowering a spoke's LTV after a supply leaves the existing
// position's snapshotted LTV unchanged; only the freshly resolved config drops.
#[test]
fn lowering_spoke_ltv_keeps_existing_position_ltv() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));

        // First supply snapshots LTV 9000 into the position.
        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let mut cache_before = Cache::new_view(&env);
        let cfg_9000: AssetConfig = (&cache_before.require_spoke_asset(1, &hub(&asset))).into();
        let seeded = account.get_or_create_supply_position(&hub(&asset), &cfg_9000);
        account.supply_positions.set(hub(&asset), (&seeded).into());

        // Governance lowers the spoke LTV to 5000 (a later transaction, hence a
        // fresh cache; the per-tx memo never serves a stale config).
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(5_000));
        let mut cache_after = Cache::new_view(&env);
        let cfg_5000: AssetConfig = (&cache_after.require_spoke_asset(1, &hub(&asset))).into();
        assert_eq!(cfg_5000.loan_to_value.raw() as u32, 5_000);

        // The existing position keeps the snapshotted 9000.
        let existing = account.get_or_create_supply_position(&hub(&asset), &cfg_5000);
        assert_eq!(existing.loan_to_value.raw() as u32, 9_000);
    });
}

// The usage-decrement sign guard fires: a decrement larger than the stored
// total is an accounting invariant breach (#34 InternalError), not a silent
// negative row that would later fake the zero-usage removal gate.
#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn usage_supply_decrement_below_zero_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_usage(
            &env,
            1,
            &hub(&asset),
            &SpokeUsageRaw {
                supplied_scaled_ray: 5,
                borrowed_scaled_ray: 0,
            },
        );
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_withdraw_after_pool(&env, &hub(&asset), Ray::from(10));
    });
}

// Borrow-side twin of the sign guard.
#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn usage_borrow_decrement_below_zero_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_usage(
            &env,
            1,
            &hub(&asset),
            &SpokeUsageRaw {
                supplied_scaled_ray: 0,
                borrowed_scaled_ray: 5,
            },
        );
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_repay_after_pool(&env, &hub(&asset), Ray::from(10));
    });
}

// Reaching the post-pool supply accounting without a listing means the entry
// gates were bypassed; the branch fails loud (#34) instead of silently
// skipping the cap check and usage increment.
#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn apply_supply_without_listing_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        let index = MarketIndexRaw {
            supply_index: RAY,
            borrow_index: RAY,
        };
        ctx.apply_supply_after_pool(&env, &hub(&asset), Ray::from(1), &index, 7);
    });
}

// Borrow-side twin of the missing-listing guard.
#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn apply_borrow_without_listing_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        let index = MarketIndexRaw {
            supply_index: RAY,
            borrow_index: RAY,
        };
        ctx.apply_borrow_after_pool(&env, &hub(&asset), Ray::from(1), &index, 7);
    });
}
