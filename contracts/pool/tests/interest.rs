extern crate std;

use super::*;
use crate::test_support::init_ledger;
use crate::{LiquidityPool, LiquidityPoolClient};
use common::constants::RAY;
use common::types::{HubAssetKey, MarketParamsRaw, PoolKey, PoolStateRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

/// Phase 0 markets all live on hub 0.
fn hub(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

struct TestSetup {
    env: Env,
    contract: Address,
    asset: Address,
}

impl TestSetup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        init_ledger(&env);

        let admin = Address::generate(&env);
        let asset = Address::generate(&env);
        let params = MarketParamsRaw {
            max_borrow_rate: 2 * RAY,
            base_borrow_rate: RAY / 100,
            slope1: RAY / 10,
            slope2: RAY / 5,
            slope3: RAY / 2,
            mid_utilization: RAY / 2,
            optimal_utilization: RAY * 8 / 10,
            max_utilization: RAY * 95 / 100,
            reserve_factor: 1_000,
            supply_cap: 0,
            borrow_cap: 0,
            is_flashloanable: false,
            flashloan_fee: 0,
            asset_id: asset.clone(),
            asset_decimals: 7,
        };
        let contract = env.register(LiquidityPool, (admin.clone(),));
        LiquidityPoolClient::new(&env, &contract).create_market(&0u32, &params);

        Self {
            env,
            contract,
            asset,
        }
    }

    fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
        self.env.as_contract(&self.contract, f)
    }

    fn fresh_cache(&self, state: PoolStateRaw) -> Cache {
        self.env
            .storage()
            .persistent()
            .set(&PoolKey::State(hub(&self.asset)), &state);
        Cache::load(&self.env, &hub(&self.asset))
    }
}

// Zero RAY fee is a no-op.
#[test]
fn test_add_protocol_revenue_ray_zero_is_noop() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let (rev_before, supp_before) = (cache.revenue, cache.supplied);
        add_protocol_revenue(&mut cache, Ray::ZERO);
        assert_eq!(cache.revenue, rev_before);
        assert_eq!(cache.supplied, supp_before);
    });
}

// Skip RAY-fee accrual at or below the supply-index floor; division by a
// near-zero index creates oversized scaled amounts.
#[test]
fn test_add_protocol_revenue_ray_skips_when_supply_index_below_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // Set supply_index to floor - 1 for the safety branch.
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: SUPPLY_INDEX_FLOOR_RAW - 1,
            last_timestamp: 0,
            cash: 0,
        });
        let (rev_before, supp_before) = (cache.revenue, cache.supplied);

        add_protocol_revenue(&mut cache, Ray::from(1_000_000));

        assert_eq!(cache.revenue, rev_before);
        assert_eq!(cache.supplied, supp_before);
    });
}

// Zero total supply short-circuits; no suppliers absorb bad debt.
#[test]
fn test_apply_bad_debt_noop_when_total_supply_is_zero() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 0,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let index_before = cache.supply_index;
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(5 * RAY));
        assert_eq!(cache.supply_index, index_before);
    });
}

// bad_debt above total supply is capped, then the floor clamp applies.
#[test]
fn test_apply_bad_debt_caps_at_total_supply_and_clamps_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 10 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY, // total supply value = 10 * RAY
            last_timestamp: 0,
            cash: 0,
        });

        // bad_debt > total_supplied: capped path plus >90% reduction.
        // The new index clamps to the floor.
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));

        assert_eq!(
            cache.supply_index.raw(),
            SUPPLY_INDEX_FLOOR_RAW,
            "supply index must be clamped to floor"
        );
    });
}

// A >90% reduction can apply without the floor clamp.
#[test]
fn test_apply_bad_debt_applies_severe_reduction() {
    let t = TestSetup::new();
    t.as_contract(|| {
        // High supply_index keeps a 91% reduction above the floor.
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 1_000 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            // Supply index ~1.0 means total_supplied_value ~= 1000*RAY.
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let old_index = cache.supply_index.raw();

        // 91% of 1000*RAY = 910*RAY; index drops below 10% of its prior value.
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(910 * RAY));

        assert!(
            cache.supply_index.raw() < old_index / 10,
            "index should have dropped more than 10x"
        );
    });
}

// Read-path simulation must match mutating accrual across multi-year deltas;
// both paths chunk at one year. Mismatch desyncs valuations from persisted state.
#[test]
fn test_simulate_matches_global_sync_over_multi_year_delta() {
    use common::rates::simulate_update_indexes;
    use common::types::PoolSyncData;

    let t = TestSetup::new();
    t.as_contract(|| {
        let state = PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        };
        let params: MarketParamsRaw = t
            .env
            .storage()
            .persistent()
            .get(&PoolKey::Params(hub(&t.asset)))
            .unwrap();
        let sync = PoolSyncData {
            params,
            state: state.clone(),
        };

        let mut cache = t.fresh_cache(state);
        // 2.5 years elapsed: three chunks (1y + 1y + 0.5y).
        let delta_ms = 2 * MAX_COMPOUND_DELTA_MS + MAX_COMPOUND_DELTA_MS / 2;
        cache.current_timestamp = cache.last_timestamp + delta_ms;
        let simulated = simulate_update_indexes(&t.env, cache.current_timestamp, &sync);

        global_sync(&t.env, &mut cache);

        assert_eq!(
            cache.borrow_index.raw(),
            simulated.borrow_index.raw(),
            "read-path borrow index must equal mutating accrual"
        );
        assert_eq!(
            cache.supply_index.raw(),
            simulated.supply_index.raw(),
            "read-path supply index must equal mutating accrual"
        );
    });
}

// A mild (<90%) reduction skips the floor clamp.
#[test]
fn test_apply_bad_debt_mild_reduction_preserves_index_above_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 1_000 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let old_index = cache.supply_index.raw();

        // 10% bad debt reduces the index and stays above floor.
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));

        let new_index = cache.supply_index.raw();
        assert!(new_index > old_index / 10, "should be a mild reduction");
        assert!(new_index > SUPPLY_INDEX_FLOOR_RAW, "should be above floor");
        assert!(new_index < old_index, "should be reduced");
    });
}

#[test]
fn test_global_sync_respects_chunk_boundary() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let state = PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 60 * RAY,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 40_000_000,
        };
        let mut cache = t.fresh_cache(state);
        // One full chunk.
        cache.current_timestamp = MAX_COMPOUND_DELTA_MS;
        global_sync(&t.env, &mut cache);
        assert!(cache.borrow_index.raw() > RAY);
    });
}

#[test]
fn test_apply_bad_debt_exactly_at_total_supplied_hits_cap_and_floor() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        apply_bad_debt_to_supply_index(&mut cache, Ray::from(100 * RAY));
        assert_eq!(cache.supply_index.raw(), SUPPLY_INDEX_FLOOR_RAW);
    });
}

#[test]
fn test_global_sync_step_zero_borrowed_produces_zero_interest() {
    let t = TestSetup::new();
    t.as_contract(|| {
        let mut cache = t.fresh_cache(PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: 0,
            cash: 0,
        });
        let before = cache.supply_index;
        // Positive delta without borrows leaves supply index unchanged.
        cache.current_timestamp = 1_000;
        global_sync(&t.env, &mut cache);
        assert_eq!(cache.supply_index, before);
    });
}
