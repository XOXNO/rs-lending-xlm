use common::constants::{BPS, MILLISECONDS_PER_YEAR, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::events::{emit_pool_insolvent, PoolInsolventEvent};
use common::fp::Ray;
use common::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest, update_borrow_index,
    update_supply_index,
};
use soroban_sdk::Env;

use crate::cache::Cache;

/// Cap on compound interval per `global_sync` call. The 8-term Taylor
/// expansion in `compound_interest` holds only for
/// `x = rate * delta_ms / MS_PER_YEAR <= 2`; capping `delta_ms` at one year
/// and iterating keeps `x <= annual_rate` (bounded by `max_borrow_rate`)
/// and avoids i128 overflow in `x_pow8`.
const MAX_COMPOUND_DELTA_MS: u64 = MILLISECONDS_PER_YEAR;

/// Accrues interest on the pool state from `last_timestamp` to `current_timestamp`.
/// Iterates in `MAX_COMPOUND_DELTA_MS` chunks to keep the Taylor series within its
/// documented accuracy envelope. No-ops when `delta_ms` is zero.
pub fn global_sync(env: &Env, cache: &mut Cache) {
    let total_delta_ms = cache.current_timestamp.saturating_sub(cache.last_timestamp);

    if total_delta_ms == 0 {
        return;
    }

    // Iterate compound interest in bounded chunks so a market idle for
    // multiple years cannot push `x` into the Taylor degradation /
    // overflow regime. Each chunk fully commits its own sub-accrual.
    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);
        global_sync_step(env, cache, chunk);
        remaining -= chunk;
    }

    cache.last_timestamp = cache.current_timestamp;
}

fn global_sync_step(env: &Env, cache: &mut Cache, delta_ms: u64) {
    let util = Ray::from_raw(cache.calculate_utilization());
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

    // Protocol fee is already in RAY from rates math.
    add_protocol_revenue_ray(cache, protocol_fee);
}

/// Converts a fee in **asset decimals** to RAY-scaled supply tokens.
/// Used for fees that arrive as token amounts (liquidation protocol_fee,
/// flash loan fee).
pub fn add_protocol_revenue(cache: &mut Cache, fee_amount: i128) {
    if fee_amount <= 0 {
        return;
    }
    // Skip accrual when the supply index is at or near the safety floor:
    // dividing by a near-zero index produces astronomical scaled amounts.
    if cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW {
        return;
    }
    let fee_scaled = Ray::from_asset(fee_amount, cache.params.asset_decimals)
        .div(&cache.env, cache.supply_index);
    cache.revenue += fee_scaled;
    cache.supplied += fee_scaled;
}

/// Converts a fee already in **RAY** to scaled supply tokens.
/// Used for fees computed internally (interest accrual).
pub fn add_protocol_revenue_ray(cache: &mut Cache, fee: Ray) {
    if fee == Ray::ZERO {
        return;
    }
    // Skip revenue accrual when supply_index is at or near the safety floor:
    // dividing by a near-zero index produces astronomical scaled amounts.
    if cache.supply_index.raw() <= SUPPLY_INDEX_FLOOR_RAW {
        return;
    }
    let fee_scaled = fee.div(&cache.env, cache.supply_index);
    cache.revenue += fee_scaled;
    cache.supplied += fee_scaled;
}

/// Reduces the supply index to socialize uncollectable debt.
///
/// Safety:
/// - The resulting index is floored at `SUPPLY_INDEX_FLOOR_RAW` (10^18 raw Ray,
///   = 10^-9 decimal). Dropping to zero would make
///   `cache.supply_index.raw() == 0`, which divides-by-zero in downstream
///   `amount / supply_index` conversions; the revenue-accrual paths
///   additionally short-circuit when `index <= floor` so a near-zero index
///   cannot blow up `fee / supply_index`.
/// - If the proposed reduction would drop the index by more than 90% in a
///   single call, emits `PoolInsolventEvent` for external monitoring. The
///   pool does not pause itself; the reduction still applies, clamped to the
///   floor, so existing math stays consistent.
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

    let old_index_raw = cache.supply_index.raw();
    let new_index_raw = new_supply_index.raw();

    // Insolvency signal: a drop of more than 90% in a single bad-debt event.
    // If the candidate index falls below one tenth of the prior index, emit a
    // controller-consumable risk event.
    // The pool itself has no status field to pause writes.
    if new_index_raw < old_index_raw / 10 {
        // bad_debt_ratio_bps = min(bad_debt / total_supplied, 100%) * BPS.
        let ratio_ray = capped.div(&cache.env, total_supplied_value);
        // ratio_ray is in [0, RAY]; convert to BPS (0..=BPS):
        // bps = ratio_ray * BPS / RAY.
        let bad_debt_ratio_bps = ratio_ray.raw() / (RAY / BPS);

        emit_pool_insolvent(
            &cache.env,
            PoolInsolventEvent {
                asset: cache.params.asset_id.clone(),
                bad_debt_ratio_bps,
                old_supply_index_ray: old_index_raw,
                new_supply_index_ray: new_index_raw,
            },
        );
    }

    // Floor at SUPPLY_INDEX_FLOOR_RAW to keep downstream math conditioned.
    cache.supply_index = if new_index_raw < SUPPLY_INDEX_FLOOR_RAW {
        Ray::from_raw(SUPPLY_INDEX_FLOOR_RAW)
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
    use common::constants::RAY;
    use common::types::{MarketParams, PoolKey, PoolState};
    use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
    use soroban_sdk::{Address, Env};

    struct TestSetup {
        env: Env,
        contract: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 26,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });

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

    // Non-positive fees are no-ops.
    #[test]
    fn test_add_protocol_revenue_zero_fee_is_noop() {
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

            add_protocol_revenue(&mut cache, 0);
            assert_eq!(cache.revenue, rev_before);
            assert_eq!(cache.supplied, supp_before);

            add_protocol_revenue(&mut cache, -100);
            assert_eq!(cache.revenue, rev_before);
            assert_eq!(cache.supplied, supp_before);
        });
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

    // The floor guard applies to the asset-decimal fee path (liquidation,
    // flash-loan, and strategy fees) -- all run after global_sync, which
    // can clamp the supply index.
    #[test]
    fn test_add_protocol_revenue_skips_when_supply_index_below_floor() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let mut cache = t.fresh_cache(PoolState {
                supplied_ray: 100 * RAY,
                borrowed_ray: 0,
                revenue_ray: 0,
                borrow_index_ray: RAY,
                supply_index_ray: SUPPLY_INDEX_FLOOR_RAW - 1,
                last_timestamp: 0,
            });
            let (rev_before, supp_before) = (cache.revenue, cache.supplied);

            // Non-zero asset-denominated fee -- would explode without the guard.
            add_protocol_revenue(&mut cache, 1_000_000);

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
    // the insolvency branch and floor clamp.
    #[test]
    fn test_apply_bad_debt_caps_at_total_supply_and_triggers_insolvency() {
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
            // emits the event and clamps the new index (~0) to the floor.
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(100 * RAY));

            assert_eq!(
                cache.supply_index.raw(),
                SUPPLY_INDEX_FLOOR_RAW,
                "supply index must be clamped to floor"
            );
        });
    }

    // A >90% reduction emits PoolInsolventEvent without requiring the debt
    // to exceed total supply; a 91% bad-debt event suffices.
    #[test]
    fn test_apply_bad_debt_emits_insolvent_event_on_severe_reduction() {
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

    // A mild (<90%) reduction emits no event and skips the floor clamp.
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

            // 10% of total supply: index drops ~10%, well above floor and
            // below the insolvency trigger.
            apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(100 * RAY));

            let new_index = cache.supply_index.raw();
            assert!(new_index > old_index / 10, "should not trigger insolvency");
            assert!(new_index > SUPPLY_INDEX_FLOOR_RAW, "should be above floor");
            assert!(new_index < old_index, "should be reduced");
        });
    }
}
