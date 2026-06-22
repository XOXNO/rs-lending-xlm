use common::constants::SUPPLY_INDEX_FLOOR_RAW;
use common::math::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, update_borrow_index,
    update_supply_index, MAX_COMPOUND_DELTA_MS,
};
use soroban_sdk::Env;

use crate::cache::Cache;

/// Accrues interest from the last pool timestamp to the current ledger timestamp.
pub fn global_sync(env: &Env, cache: &mut Cache) {
    let total_delta_ms = cache.current_timestamp.saturating_sub(cache.last_timestamp);

    if total_delta_ms == 0 {
        return;
    }

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);
        global_sync_step(env, cache, chunk);
        remaining = remaining.saturating_sub(chunk);
    }

    cache.last_timestamp = cache.current_timestamp;
}

fn global_sync_step(env: &Env, cache: &mut Cache, delta_ms: u64) {
    let util = cache.calculate_utilization();
    let borrow_rate = calculate_borrow_rate(env, util, &cache.params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let new_borrow_index = update_borrow_index(env, cache.borrow_index, interest_factor);

    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        env,
        &cache.params,
        cache.borrowed,
        new_borrow_index,
        cache.borrow_index,
    );

    let new_supply_index =
        update_supply_index(env, cache.supplied, cache.supply_index, supplier_rewards);

    cache.borrow_index = new_borrow_index;
    cache.supply_index = new_supply_index;

    // Protocol fee is added to revenue and scaled supplied; later chunks in the
    // same accrual use diluted utilization.
    add_protocol_revenue_ray(cache, protocol_fee);
}

/// Adds a RAY-denominated fee as scaled protocol revenue.
pub fn add_protocol_revenue_ray(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }
    if cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW {
        return;
    }
    // Fees on an empty pool are dropped; there are no suppliers to dilute.
    if cache.supplied == Ray::ZERO {
        return;
    }
    let fee_scaled = fee.div(&cache.env, cache.supply_index);
    cache.revenue.checked_add_assign(&cache.env, fee_scaled);
    cache.supplied.checked_add_assign(&cache.env, fee_scaled);
}

/// Socializes uncollectable debt by reducing the supply index.
pub fn apply_bad_debt_to_supply_index(cache: &mut Cache, bad_debt: Ray) {
    let total_supplied_value = cache.supplied.mul(&cache.env, cache.supply_index);

    if total_supplied_value == Ray::ZERO {
        return;
    }

    let capped = if bad_debt > total_supplied_value {
        total_supplied_value
    } else {
        bad_debt
    };
    let remaining = total_supplied_value - capped;

    let reduction_factor = remaining.div(&cache.env, total_supplied_value);
    let new_supply_index = cache.supply_index.mul(&cache.env, reduction_factor);

    let floor_index = Ray::from(SUPPLY_INDEX_FLOOR_RAW);

    cache.supply_index = if new_supply_index < floor_index {
        floor_index
    } else {
        new_supply_index
    };
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::test_support::init_ledger;
    use crate::{LiquidityPool, LiquidityPoolClient};
    use common::constants::RAY;
    use common::types::{MarketParamsRaw, PoolKey, PoolStateRaw};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

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
                max_borrow_rate_ray: 2 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                max_utilization_ray: RAY * 95 / 100,
                reserve_factor_bps: 1_000,
                supply_cap: 0,
                borrow_cap: 0,
                asset_id: asset.clone(),
                asset_decimals: 7,
            };
            let contract = env.register(LiquidityPool, (admin.clone(),));
            LiquidityPoolClient::new(&env, &contract).create_market(&params);

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
                .set(&PoolKey::State(self.asset.clone()), &state);
            Cache::load(&self.env, &self.asset)
        }
    }

    // Zero RAY fee is a no-op.
    #[test]
    fn test_add_protocol_revenue_ray_zero_is_noop() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolStateRaw {
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
                last_timestamp: 0,
                cash: 0,
            });
            let (rev_before, supp_before) = (cache.revenue, cache.supplied);
            add_protocol_revenue_ray(&mut cache, Ray::ZERO);
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
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: SUPPLY_INDEX_FLOOR_RAW - 1,
                last_timestamp: 0,
                cash: 0,
            });
            let (rev_before, supp_before) = (cache.revenue, cache.supplied);

            add_protocol_revenue_ray(&mut cache, Ray::from(1_000_000));

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
                supplied_ray: 0,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
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
                supplied_ray: 10 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY, // total supply value = 10 * RAY
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
                supplied_ray: 1_000 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                // Supply index ~1.0 means total_supplied_value ~= 1000*RAY.
                supply_index_ray: RAY,
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
                supplied_ray: 100 * RAY,
                borrowed_ray: 60 * RAY,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
                last_timestamp: 0,
                cash: 40_000_000,
            };
            let params: MarketParamsRaw = t
                .env
                .storage()
                .persistent()
                .get(&PoolKey::Params(t.asset.clone()))
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
                supplied_ray: 1_000 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
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
                supplied_ray: 100 * RAY,
                borrowed_ray: 60 * RAY,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
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
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
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
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
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
}
