use common::constants::{MILLISECONDS_PER_YEAR, SUPPLY_INDEX_FLOOR_RAW};
use common::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, update_borrow_index,
    update_supply_index,
};
use soroban_sdk::Env;

use crate::cache::Cache;

/// Per-chunk cap. `compound_interest`'s 8-term Taylor expansion holds only
/// for `x = rate * delta_ms / MS_PER_YEAR <= 2`; one year keeps `x` bounded
/// by `max_borrow_rate` and avoids i128 overflow in `x_pow8`.
const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

/// Accrues interest from `last_timestamp` to `current_timestamp`, iterating
/// in `MAX_COMPOUND_DELTA_MS` chunks to stay inside the Taylor envelope.
/// No-op when `delta_ms == 0`.
pub fn global_sync(env: &Env, cache: &mut Cache) {
    let total_delta_ms = cache.current_timestamp.saturating_sub(cache.last_timestamp);

    if total_delta_ms == 0 {
        return;
    }

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);
        global_sync_step(env, cache, chunk);
        remaining -= chunk;
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

    add_protocol_revenue_ray(cache, protocol_fee);
}

/// Accrues a RAY-denominated fee to protocol revenue. Asset-decimal callers
/// must convert via `Ray::from_asset` first. Skips when `supply_index` is at
/// or below the floor (division would blow up).
pub fn add_protocol_revenue_ray(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }
    if cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW {
        return;
    }
    let fee_scaled = fee.div(&cache.env, cache.supply_index);
    cache.revenue += fee_scaled;
    cache.supplied += fee_scaled;
}

/// Socialises uncollectable debt by reducing the supply index.
///
/// The new index is floored at `SUPPLY_INDEX_FLOOR_RAW` (10^-9 decimal):
/// a zero index divides-by-zero in `amount / supply_index` conversions, and
/// revenue accrual short-circuits when `index <= floor`. The pool does not
/// auto-pause on severe reductions; the controller emits the cleanup event
/// and market snapshot for off-chain monitoring.
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

    let floor_index = Ray::from_raw(SUPPLY_INDEX_FLOOR_RAW);

    cache.supply_index = if new_supply_index < floor_index {
        floor_index
    } else {
        new_supply_index
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::test_support::init_ledger;
    use common::constants::RAY;
    use common::types::{MarketParams, PoolKey, PoolState};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    struct TestSetup {
        env: Env,
        contract: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            init_ledger(&env);

            let admin = Address::generate(&env);
            let params = MarketParams {
                max_borrow_rate_ray: 5 * RAY,
                base_borrow_rate_ray: RAY / 100,
                slope1_ray: RAY / 10,
                slope2_ray: RAY / 5,
                slope3_ray: RAY / 2,
                mid_utilization_ray: RAY / 2,
                optimal_utilization_ray: RAY * 8 / 10,
                reserve_factor_bps: 1_000,
                asset_id: Address::generate(&env),
                asset_decimals: 7,
            };
            let contract = env.register(crate::LiquidityPool, (admin.clone(), params));

            Self { env, contract }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }

        fn fresh_cache(&self, state: PoolState) -> Cache {
            self.env.storage().instance().set(&PoolKey::State, &state);
            Cache::load(&self.env)
        }
    }

    // Zero RAY fee is a no-op.
    #[test]
    fn test_add_protocol_revenue_ray_zero_is_noop() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
                last_timestamp: 0,
            });
            let (rev_before, supp_before) = (cache.revenue, cache.supplied);
            add_protocol_revenue_ray(&mut cache, Ray::ZERO);
            assert_eq!(cache.revenue, rev_before);
            assert_eq!(cache.supplied, supp_before);
        });
    }

    // Skips RAY-fee accrual when supply_index is at or below the safety floor;
    // dividing by a near-zero index produces astronomical scaled amounts.
    #[test]
    fn test_add_protocol_revenue_ray_skips_when_supply_index_below_floor() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // supply_index set to (floor - 1) to trigger the safety branch.
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: SUPPLY_INDEX_FLOOR_RAW - 1,
                last_timestamp: 0,
            });
            let (rev_before, supp_before) = (cache.revenue, cache.supplied);

            add_protocol_revenue_ray(&mut cache, Ray::from_raw(1_000_000));

            assert_eq!(cache.revenue, rev_before);
            assert_eq!(cache.supplied, supp_before);
        });
    }

    // Zero total supply short-circuits; nothing to socialize against.
    #[test]
    fn test_apply_bad_debt_noop_when_total_supply_is_zero() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 0,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
                last_timestamp: 0,
            });
            let index_before = cache.supply_index;
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(5 * RAY));
            assert_eq!(cache.supply_index, index_before);
        });
    }

    // When bad_debt > total_supplied, the cap clamps the debt and triggers
    // the floor clamp.
    #[test]
    fn test_apply_bad_debt_caps_at_total_supply_and_clamps_floor() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 10 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY, // total supply value = 10 * RAY
                last_timestamp: 0,
            });

            // bad_debt > total_supplied -> capped path + >90% reduction
            // clamps the new index (~0) to the floor.
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(100 * RAY));

            assert_eq!(
                cache.supply_index.raw(),
                SUPPLY_INDEX_FLOOR_RAW,
                "supply index must be clamped to floor"
            );
        });
    }

    // A >90% reduction still applies and can be observed through the
    // controller's market batch snapshot.
    #[test]
    fn test_apply_bad_debt_applies_severe_reduction() {
        let t = TestSetup::new();
        t.as_contract(|| {
            // supply_index high enough that a 91% drop still leaves the new
            // index above the floor, exercising the non-clamping branch.
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 1_000 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                // Supply index ~1.0 means total_supplied_value ~= 1000*RAY.
                supply_index_ray: RAY,
                last_timestamp: 0,
            });
            let old_index = cache.supply_index.raw();

            // 91% of 1000*RAY = 910*RAY bad debt; index collapses >10x.
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(910 * RAY));

            assert!(
                cache.supply_index.raw() < old_index / 10,
                "index should have dropped more than 10x"
            );
        });
    }

    // A mild (<90%) reduction skips the floor clamp.
    #[test]
    fn test_apply_bad_debt_mild_reduction_preserves_index_above_floor() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 1_000 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: RAY,
                last_timestamp: 0,
            });
            let old_index = cache.supply_index.raw();

            // 10% of total supply: index drops ~10% and stays well above floor.
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(100 * RAY));

            let new_index = cache.supply_index.raw();
            assert!(new_index > old_index / 10, "should be a mild reduction");
            assert!(new_index > SUPPLY_INDEX_FLOOR_RAW, "should be above floor");
            assert!(new_index < old_index, "should be reduced");
        });
    }
}
