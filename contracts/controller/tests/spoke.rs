use super::*;
use crate::context::Context;
use crate::storage;
use crate::Controller;
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{
    Account, AssetConfig, HubAssetKey, MarketIndexRaw, PositionMode, SpokeAssetConfig,
    SpokeUsageRaw,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Map};

fn spoke_asset_config(ltv_bps: u32) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        no_seize: false,
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

#[test]
fn require_spoke_asset_converts_listed_risk_config() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Context::new_view(&env);
        let cfg: AssetConfig = cache.require_spoke_asset(1, &hub(&asset));
        assert_eq!(cfg.loan_to_value.raw() as u32, 9_000);
        assert!(cfg.is_collateralizable);
        assert!(cfg.is_borrowable);
    });
}

#[test]
fn require_spoke_asset_reads_each_spoke_directly() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        storage::set_spoke_asset(&env, 2, &hub(&asset), &spoke_asset_config(5_000));

        let mut cache_spoke_1 = Context::new_view(&env);
        let cfg1: AssetConfig = cache_spoke_1.require_spoke_asset(1, &hub(&asset));
        assert_eq!(cfg1.loan_to_value.raw() as u32, 9_000);
        let mut cache_spoke_2 = Context::new_view(&env);
        let cfg2: AssetConfig = cache_spoke_2.require_spoke_asset(2, &hub(&asset));
        assert_eq!(cfg2.loan_to_value.raw() as u32, 5_000);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn require_spoke_asset_panics_when_unlisted_on_spoke() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));
        let mut cache = Context::new_view(&env);
        let _: AssetConfig = cache.require_spoke_asset(2, &hub(&asset));
    });
}

#[test]
fn lowering_spoke_ltv_keeps_existing_position_ltv() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(9_000));

        let mut account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions: Map::new(&env),
        };
        let mut cache_before = Context::new_view(&env);
        let cfg_9000: AssetConfig = cache_before.require_spoke_asset(1, &hub(&asset));
        let seeded = account.get_or_create_supply_position(&hub(&asset), &cfg_9000);
        account.supply_positions.set(hub(&asset), (&seeded).into());

        storage::set_spoke_asset(&env, 1, &hub(&asset), &spoke_asset_config(5_000));
        let mut cache_after = Context::new_view(&env);
        let cfg_5000: AssetConfig = cache_after.require_spoke_asset(1, &hub(&asset));
        assert_eq!(cfg_5000.loan_to_value.raw() as u32, 5_000);

        let existing = account.get_or_create_supply_position(&hub(&asset), &cfg_5000);
        assert_eq!(existing.loan_to_value.raw() as u32, 9_000);
    });
}

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
        ctx.apply_exit(UsageSide::Supply, &hub(&asset), Ray::from(10));
    });
}

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
        ctx.apply_exit(UsageSide::Borrow, &hub(&asset), Ray::from(10));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #311)")]
fn zero_supply_cap_rejects_entry() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &hub(&asset),
            Ray::from(RAY),
            0,
            Ray::from(RAY),
            7,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #312)")]
fn zero_borrow_cap_rejects_entry() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Borrow,
            &hub(&asset),
            Ray::from(RAY),
            0,
            Ray::from(RAY),
            7,
        );
    });
}

#[test]
fn apply_entry_stores_single_add_not_dual_add() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    let key = hub(&asset);
    // decimals=7: 1 asset unit as ray = 10^(27-7). Cap is asset units; usage is scaled ray.
    let unit = Ray::from_asset(&env, 1, 7).raw();
    let prior = 3 * unit;
    let delta = 2 * unit;
    let cap_asset = 10;
    env.as_contract(&contract, || {
        storage::set_spoke_usage(
            &env,
            1,
            &key,
            &SpokeUsageRaw {
                supplied_scaled_ray: prior,
                borrowed_scaled_ray: 0,
            },
        );
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &key,
            Ray::from(delta),
            cap_asset,
            Ray::from(RAY),
            7,
        );
        ctx.persist();

        let stored = storage::get_spoke_usage(&env, 1, &key).expect("usage row");
        assert_eq!(
            stored.supplied_scaled_ray,
            prior + delta,
            "apply_entry must store usage + delta once; dual-add would write prior + 2*delta"
        );
        assert_eq!(stored.borrowed_scaled_ray, 0);
    });
}

#[test]
fn apply_entry_at_exact_cap_succeeds() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    let key = hub(&asset);
    // Cap 5 asset units at index RAY → scaled = from_asset(env, 5, 7).
    let cap_asset = 5;
    let delta = Ray::from_asset(&env, 5, 7).raw();
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &key,
            Ray::from(delta),
            cap_asset,
            Ray::from(RAY),
            7,
        );
        ctx.persist();
        let stored = storage::get_spoke_usage(&env, 1, &key).expect("usage row");
        assert_eq!(stored.supplied_scaled_ray, delta);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #311)")]
fn apply_entry_one_over_cap_reverts_with_supply_cap() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        // Cap 1 asset unit; attempt 1 asset unit + 1 scaled ray dust.
        ctx.apply_entry(
            UsageSide::Supply,
            &hub(&asset),
            Ray::from(RAY + 1),
            1,
            Ray::from(RAY),
            7,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn apply_entry_overflow_on_usage_plus_delta_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    let key = hub(&asset);
    env.as_contract(&contract, || {
        storage::set_spoke_usage(
            &env,
            1,
            &key,
            &SpokeUsageRaw {
                supplied_scaled_ray: i128::MAX,
                borrowed_scaled_ray: 0,
            },
        );
        let mut ctx = SpokeUsageContext::new(&env, 1);
        // Cap is domain ceiling so the overflow path is hit before (or instead of) cap breach.
        ctx.apply_entry(
            UsageSide::Supply,
            &key,
            Ray::from(1),
            common::validation::max_cap_for_decimals(7),
            Ray::from(RAY),
            7,
        );
    });
}

#[test]
fn ceiling_cap_saturates_instead_of_panicking_at_the_index_floor() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &hub(&asset),
            Ray::from(RAY),
            common::validation::max_cap_for_decimals(7),
            Ray::from(common::constants::SUPPLY_INDEX_FLOOR_RAW),
            7,
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn apply_supply_without_listing_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut cache = Context::new_view(&env);
        let index = MarketIndexRaw {
            supply_index: RAY,
            borrow_index: RAY,
        };
        cache.apply_spoke_entry(1, UsageSide::Supply, &hub(&asset), Ray::from(1), &index, 7);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #307)")]
fn apply_borrow_without_listing_panics() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let mut cache = Context::new_view(&env);
        let index = MarketIndexRaw {
            supply_index: RAY,
            borrow_index: RAY,
        };
        cache.apply_spoke_entry(1, UsageSide::Borrow, &hub(&asset), Ray::from(1), &index, 7);
    });
}

/// Exit path must not invent a zero usage row when storage is empty.
/// Positive delta against a missing row is a silent no-op (not InternalError).
#[test]
fn exit_without_usage_row_is_noop_and_does_not_persist() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let hub_asset = hub(&asset);
        assert!(storage::get_spoke_usage(&env, 1, &hub_asset).is_none());

        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_exit(UsageSide::Supply, &hub_asset, Ray::from(RAY));
        ctx.apply_exit(UsageSide::Borrow, &hub_asset, Ray::from(RAY));
        ctx.persist();

        assert!(
            storage::get_spoke_usage(&env, 1, &hub_asset).is_none(),
            "exit no-insert must not materialize a storage row for a missing usage key"
        );
    });
}

/// Entry path must default-insert a zero row so first supply/borrow can accrue.
#[test]
fn entry_without_usage_row_default_inserts_and_persists() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let hub_asset = hub(&asset);
        assert!(storage::get_spoke_usage(&env, 1, &hub_asset).is_none());

        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &hub_asset,
            Ray::from(RAY),
            common::validation::max_cap_for_decimals(7),
            Ray::from(RAY),
            7,
        );
        ctx.persist();

        let stored = storage::get_spoke_usage(&env, 1, &hub_asset).expect("entry must write usage");
        assert_eq!(stored.supplied_scaled_ray, RAY);
        assert_eq!(stored.borrowed_scaled_ray, 0);
    });
}

/// Same-context entry then exit must see the default-inserted/updated row,
/// not re-load as absent.
#[test]
fn exit_sees_entry_cached_row_in_same_context() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let hub_asset = hub(&asset);
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Supply,
            &hub_asset,
            Ray::from(10),
            common::validation::max_cap_for_decimals(7),
            Ray::from(RAY),
            7,
        );
        // No intermediate persist: exit must hit the in-memory map.
        ctx.apply_exit(UsageSide::Supply, &hub_asset, Ray::from(4));
        ctx.persist();

        let stored = storage::get_spoke_usage(&env, 1, &hub_asset).expect("residual usage");
        assert_eq!(stored.supplied_scaled_ray, 6);
        assert_eq!(stored.borrowed_scaled_ray, 0);
    });
}

#[test]
fn usage_side_cap_reads_matching_field() {
    let cfg = SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        no_seize: false,
        loan_to_value: 9_000,
        liquidation_threshold: 9_300,
        liquidation_bonus: 300,
        liquidation_fees: 0,
        supply_cap: 10,
        borrow_cap: 20,
    };
    assert_eq!(UsageSide::Supply.cap(&cfg), 10);
    assert_eq!(UsageSide::Borrow.cap(&cfg), 20);
}

#[test]
fn spoke_usage_context_preserves_spoke_id() {
    let env = Env::default();
    let ctx = SpokeUsageContext::new(&env, 7);
    assert_eq!(ctx.spoke_id(), 7);
}

/// Full exit of an entry-created row prunes storage via set_spoke_usage zeros.
#[test]
fn full_exit_after_entry_prunes_storage() {
    let env = Env::default();
    let contract = new_controller(&env);
    let asset = Address::generate(&env);
    env.as_contract(&contract, || {
        let hub_asset = hub(&asset);
        let mut ctx = SpokeUsageContext::new(&env, 1);
        ctx.apply_entry(
            UsageSide::Borrow,
            &hub_asset,
            Ray::from(7),
            common::validation::max_cap_for_decimals(7),
            Ray::from(RAY),
            7,
        );
        ctx.apply_exit(UsageSide::Borrow, &hub_asset, Ray::from(7));
        ctx.persist();

        assert!(
            storage::get_spoke_usage(&env, 1, &hub_asset).is_none(),
            "zero residual usage must be pruned on persist"
        );
    });
}
