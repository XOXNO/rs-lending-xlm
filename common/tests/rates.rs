use super::*;
use crate::constants::RAY;
use crate::math::fp_core::div_by_int_half_up;
use soroban_sdk::Env;

fn make_test_params() -> MarketParams {
    MarketParams {
        base_borrow_rate: Ray::from(RAY / 100),         // 1%
        slope1: Ray::from(RAY * 4 / 100),               // 4%
        slope2: Ray::from(RAY * 10 / 100),              // 10%
        slope3: Ray::from(RAY * 300 / 100),             // 300%
        mid_utilization: Ray::from(RAY * 50 / 100),     // 50%
        optimal_utilization: Ray::from(RAY * 80 / 100), // 80%
        max_utilization: Ray::from(RAY * 95 / 100),     // 95%
        max_borrow_rate: Ray::from(RAY),                // 100%
        reserve_factor: Bps::from(1000),                // 10%
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &Env::default(),
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    }
}

#[test]
fn test_borrow_rate_region1() {
    let env = Env::default();
    let params = make_test_params();

    let rate = calculate_borrow_rate(&env, Ray::ZERO, &params);
    let expected = div_by_int_half_up(RAY / 100, MILLISECONDS_PER_YEAR as i128);
    assert_eq!(rate.raw(), expected);

    let util_25 = Ray::from(RAY * 25 / 100);
    let rate = calculate_borrow_rate(&env, util_25, &params);
    let expected_annual = RAY * 3 / 100;
    let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region2() {
    let env = Env::default();
    let params = make_test_params();

    let util_50 = Ray::from(RAY * 50 / 100);
    let rate = calculate_borrow_rate(&env, util_50, &params);
    let expected_annual = RAY * 5 / 100;
    let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_65 = Ray::from(RAY * 65 / 100);
    let rate = calculate_borrow_rate(&env, util_65, &params);
    let expected_annual = RAY * 10 / 100;
    let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region3() {
    let env = Env::default();
    let params = make_test_params();

    let util_80 = Ray::from(RAY * 80 / 100);
    let rate = calculate_borrow_rate(&env, util_80, &params);
    let expected_annual = RAY * 15 / 100;
    let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_90 = Ray::from(RAY * 90 / 100);
    let rate = calculate_borrow_rate(&env, util_90, &params);
    let expected_annual = RAY;
    let expected_per_ms = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_capped() {
    let env = Env::default();
    let params = make_test_params();

    let rate = calculate_borrow_rate(&env, Ray::ONE, &params);
    let expected = div_by_int_half_up(params.max_borrow_rate.raw(), MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected).abs() <= 1);
}

#[test]
fn test_compound_interest_zero_delta() {
    let env = Env::default();
    let result = compound_interest(&env, Ray::from(RAY / 10), 0);
    assert_eq!(result, Ray::ONE);
}

#[test]
fn test_compound_interest_accuracy() {
    let env = Env::default();

    let annual_rate = Ray::from(RAY / 10);
    let rate_per_ms = annual_rate.div_by_int(MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&env, rate_per_ms, MILLISECONDS_PER_YEAR);

    let expected_e_010 = 1_105_170_918_075_647_624_811_707_826_i128;

    let diff = (factor.raw() - expected_e_010).abs();
    let tolerance = expected_e_010 / 1_000_000;
    assert!(
        diff < tolerance,
        "Compound interest accuracy: factor={}, expected={}, diff={}, tolerance={}",
        factor.raw(),
        expected_e_010,
        diff,
        tolerance
    );
}

#[test]
fn test_update_borrow_index() {
    let env = Env::default();
    let old_index = Ray::ONE;
    let factor = Ray::from(RAY + RAY * 5 / 100);
    let new_index = update_borrow_index(&env, old_index, factor);
    let expected = RAY * 105 / 100;
    assert!((new_index.raw() - expected).abs() <= 1);
}

#[test]
fn test_update_supply_index() {
    let env = Env::default();
    let supplied = Ray::from(100 * RAY);
    let old_index = Ray::ONE;
    let rewards = Ray::from(5 * RAY);
    let new_index = update_supply_index(&env, supplied, old_index, rewards);
    let expected = RAY * 105 / 100;
    assert!((new_index.raw() - expected).abs() <= 1);
}

#[test]
fn test_update_supply_index_zero_supplied() {
    let env = Env::default();
    let result = update_supply_index(&env, Ray::ZERO, Ray::ONE, Ray::from(5 * RAY));
    assert_eq!(result, Ray::ONE);
}

#[test]
fn test_utilization_basic() {
    let env = Env::default();
    let util = utilization(&env, Ray::from(50 * RAY), Ray::from(100 * RAY));
    let expected = RAY / 2;
    assert!((util.raw() - expected).abs() <= 1);
}

#[test]
fn test_utilization_zero_supplied() {
    let env = Env::default();
    assert_eq!(utilization(&env, Ray::from(50 * RAY), Ray::ZERO), Ray::ZERO);
}

#[test]
fn test_scaled_to_original() {
    let env = Env::default();
    let scaled = Ray::from(100 * RAY);
    let index = Ray::from(RAY * 105 / 100);
    let result = scaled_to_original(&env, scaled, index);
    let expected = 105 * RAY;
    assert!((result.raw() - expected).abs() <= 1);
}

#[test]
fn test_calculate_supplier_rewards() {
    let env = Env::default();
    let params = make_test_params();

    let borrowed = Ray::from(100 * RAY);
    let old_index = Ray::ONE;
    let new_index = Ray::from(RAY + RAY / 100);

    let (rewards, fee) = calculate_supplier_rewards(&env, &params, borrowed, new_index, old_index);

    let expected_fee = RAY / 10;
    let expected_rewards = RAY * 9 / 10;

    assert!(
        (fee.raw() - expected_fee).abs() <= 1,
        "fee={}, expected={}",
        fee.raw(),
        expected_fee
    );
    assert!(
        (rewards.raw() - expected_rewards).abs() <= 1,
        "rewards={}, expected={}",
        rewards.raw(),
        expected_rewards
    );
}

#[test]
fn test_deposit_rate() {
    let env = Env::default();
    let util_80 = Ray::from(RAY * 80 / 100);
    let borrow_rate = Ray::from(RAY * 5 / 100);
    let reserve_factor = Bps::from(1000);

    let rate = calculate_deposit_rate(&env, util_80, borrow_rate, reserve_factor);

    let expected = RAY * 36 / 1000;
    assert!(
        (rate.raw() - expected).abs() <= 1,
        "rate={}, expected={}",
        rate.raw(),
        expected
    );
}

#[test]
fn test_deposit_rate_zero_util() {
    let env = Env::default();
    assert_eq!(
        calculate_deposit_rate(&env, Ray::ZERO, Ray::from(RAY / 10), Bps::from(1000)),
        Ray::ZERO
    );
}

// `update_borrow_index` boundary: `new_index > MAX` clamps, `== MAX` returns
// MAX. Differentiates `>` from `==`/`>=` at the ceiling.

#[test]
fn test_update_borrow_index_at_max_does_not_panic() {
    let env = Env::default();
    let old_index = Ray::from(MAX_BORROW_INDEX_RAY);
    let new_index = update_borrow_index(&env, old_index, Ray::ONE);
    assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
}

#[test]
fn test_update_borrow_index_above_max_clamps() {
    let env = Env::default();
    let old_index = Ray::from(MAX_BORROW_INDEX_RAY);
    // factor = 1 + 1 ulp; product exceeds MAX. Accrual clamps the index at
    // the ceiling instead of panicking.
    let factor = Ray::from(RAY + 1);
    let new_index = update_borrow_index(&env, old_index, factor);
    assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
}

// `simulate_update_indexes` early-return guard `if delta_ms == 0`: nonzero
// delta plus live borrows accrues interest. Mutating `==` to `!=` returns
// the input indexes unchanged; borrow index growth distinguishes the paths.
#[test]
fn test_simulate_update_indexes_nonzero_delta_accrues() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    let params = MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    };
    let state = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 60 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 0,
        cash: 40_000_000,
    };
    let sync = PoolSyncData { params, state };

    // delta_ms > 0 accrues interest.
    let one_year = MILLISECONDS_PER_YEAR;
    let indexes = simulate_update_indexes(&env, one_year, &sync);
    assert!(
        indexes.borrow_index.raw() > RAY,
        "borrow index must grow over a nonzero delta; got {}",
        indexes.borrow_index.raw()
    );
    assert!(
        indexes.supply_index.raw() > RAY,
        "supply index must grow over a nonzero delta; got {}",
        indexes.supply_index.raw()
    );
}

// Multi-year deltas accrue through the 1-year chunk loop. A single
// 8-term Taylor evaluation at x = 2 years underestimates e^x because each
// omitted term is positive; the chunked result is greater.
#[test]
fn test_simulate_update_indexes_multi_year_exceeds_single_shot() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    let params = MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    };
    let state = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 90 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 0,
        cash: 10_000_000,
    };
    let p = MarketParams::from(&params);
    let s = PoolState::from(&state);
    let sync = PoolSyncData { params, state };

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let chunked = simulate_update_indexes(&env, two_years, &sync);

    // Single Taylor evaluation across the full delta.
    let util = utilization(
        &env,
        scaled_to_original(&env, s.borrowed, s.borrow_index),
        scaled_to_original(&env, s.supplied, s.supply_index),
    );
    let rate = calculate_borrow_rate(&env, util, &p);
    let single_shot = update_borrow_index(
        &env,
        s.borrow_index,
        compound_interest(&env, rate, two_years),
    );

    assert!(
        chunked.borrow_index.raw() > single_shot.raw(),
        "chunked 2y accrual {} must exceed single-shot {}",
        chunked.borrow_index.raw(),
        single_shot.raw()
    );
    // A 90%-utilization market over two years compounds past the
    // single-year index.
    let one_year = simulate_update_indexes(&env, MILLISECONDS_PER_YEAR, &sync);
    assert!(chunked.borrow_index.raw() > one_year.borrow_index.raw());
}

// Compares compound_interest with e^0.5. Tolerance detects a sign flip on
// any Taylor term (term2..term8). Truncation
// bound at x = 0.5 is x^9/9! ≈ 5.4e-9 → 5.4e18 in Ray units.
#[test]
fn test_compound_interest_high_x_pins_all_taylor_terms() {
    let env = Env::default();
    // rate * delta = x = 0.5 Ray. Set rate = 0.5 RAY/ms, delta = 1.
    let rate = Ray::from(RAY / 2);
    let result = compound_interest(&env, rate, 1);

    // e^0.5 = 1.6487212707001281468486507878...
    let expected = 1_648_721_270_700_128_146_848_650_787_i128;

    // Tolerance is greater than Taylor truncation (5.4e18) and smaller
    // than any single term's magnitude. Smallest relevant term is term8 ≈ 1.9e20.
    let tolerance = 1e19 as i128;
    let diff = (result.raw() - expected).abs();
    assert!(
        diff <= tolerance,
        "compound_interest(0.5) drift {} exceeds tolerance {}; got {}, expected {}",
        diff,
        tolerance,
        result.raw(),
        expected
    );
}

// `calculate_borrow_rate`'s `utilization < mid_utilization` branch boundary:
// at utilization == mid_utilization the correct (slope2) branch adds zero
// contribution, while the mutant (`<=`) branch falls into the slope1 branch
// and round-trips `mid_utilization.mul(slope1).div(mid_utilization)`. That
// round-trip is not always an exact identity under half-up fixed-point
// rounding, but the drift is usually too small (~1 raw unit) to survive the
// final per-millisecond division. This slope1 value was chosen so the
// drift lands on a rounding boundary of `MILLISECONDS_PER_YEAR` and remains
// observable in the returned rate.
#[test]
fn test_calculate_borrow_rate_mid_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params();
    params.mid_utilization = Ray::from(RAY / 3);
    params.slope1 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization + Ray::from(RAY / 5);

    let rate = calculate_borrow_rate(&env, params.mid_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == mid_utilization must take the slope2 branch (zero contribution)"
    );
}

// `calculate_borrow_rate`'s `utilization < optimal_utilization` branch
// boundary: at utilization == optimal_utilization the correct (slope3)
// branch adds zero contribution, while the mutant (`<=`) branch falls into
// the slope2 branch and round-trips `range.mul(slope2).div(range)`. slope1
// is zeroed so the region2 sum reduces to `base + slope2`, matching the
// boundary fixture used above so the same rounding-boundary drift applies.
#[test]
fn test_calculate_borrow_rate_optimal_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params();
    params.mid_utilization = Ray::from(RAY / 5);
    params.slope1 = Ray::ZERO;
    params.slope2 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization + Ray::from(RAY / 3);

    let rate = calculate_borrow_rate(&env, params.optimal_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == optimal_utilization must take the slope3 branch (zero contribution)"
    );
}

// Independently replays `simulate_update_indexes_body`'s per-chunk accrual
// and fee-reinvestment guard using only the public rate primitives. The
// guard mirrors `add_protocol_revenue`'s early-return checks in
// `contracts/pool/src/interest.rs`; comparing against this oracle lets each
// test below assert exact equality on the real (mutation-tested) guard.
fn oracle_accrual(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    mut supplied: Ray,
    mut borrow_index: Ray,
    mut supply_index: Ray,
    chunks_ms: &[u64],
) -> (Ray, Ray) {
    for &chunk in chunks_ms {
        let borrowed_orig = scaled_to_original(env, borrowed, borrow_index);
        let supplied_orig = scaled_to_original(env, supplied, supply_index);
        let util = utilization(env, borrowed_orig, supplied_orig);
        let rate = calculate_borrow_rate(env, util, params);
        let factor = compound_interest(env, rate, chunk);
        let new_borrow_index = update_borrow_index(env, borrow_index, factor);
        let (supplier_rewards, protocol_fee) =
            calculate_supplier_rewards(env, params, borrowed, new_borrow_index, borrow_index);
        supply_index = update_supply_index(env, supplied, supply_index, supplier_rewards);
        borrow_index = new_borrow_index;

        if protocol_fee != Ray::ZERO
            && supply_index.raw() > SUPPLY_INDEX_FLOOR_RAW
            && supplied != Ray::ZERO
        {
            let fee_scaled = protocol_fee.div(env, supply_index);
            supplied = supplied.checked_add(env, fee_scaled);
        }
    }
    (borrow_index, supply_index)
}

#[test]
fn test_simulate_guard_reinvests_fee_when_healthy() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    let raw_params = MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    };
    let raw_state = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 60 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 0,
        cash: 40_000_000,
    };
    let params = MarketParams::from(&raw_params);
    let sync = PoolSyncData {
        params: raw_params,
        state: raw_state,
    };

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::from(100 * RAY),
        Ray::ONE,
        Ray::ONE,
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}

#[test]
fn test_simulate_guard_skips_reinvestment_at_supply_index_floor() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    // reserve_factor = 100% keeps supplier_rewards at exactly zero each
    // chunk, so `update_supply_index` short-circuits and supply_index stays
    // pinned at the floor -- isolating the `supply_index > FLOOR` clause.
    let raw_params = MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 10_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    };
    let raw_state = PoolStateRaw {
        supplied: 100 * RAY,
        borrowed: 60 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: SUPPLY_INDEX_FLOOR_RAW,
        last_timestamp: 0,
        cash: 40_000_000,
    };
    let params = MarketParams::from(&raw_params);
    let sync = PoolSyncData {
        params: raw_params,
        state: raw_state,
    };

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::from(100 * RAY),
        Ray::ONE,
        Ray::from(SUPPLY_INDEX_FLOOR_RAW),
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}

#[test]
fn test_simulate_guard_skips_reinvestment_when_supplied_zero() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    let raw_params = MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: RAY / 100,
        slope1: RAY * 4 / 100,
        slope2: RAY * 10 / 100,
        slope3: RAY * 300 / 100,
        mid_utilization: RAY * 50 / 100,
        optimal_utilization: RAY * 80 / 100,
        max_utilization: RAY * 95 / 100,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: soroban_sdk::Address::from_str(
            &env,
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    };
    let raw_state = PoolStateRaw {
        supplied: 0,
        borrowed: 60 * RAY,
        revenue: 0,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: 0,
        cash: 40_000_000,
    };
    let params = MarketParams::from(&raw_params);
    let sync = PoolSyncData {
        params: raw_params,
        state: raw_state,
    };

    let two_years = 2 * MILLISECONDS_PER_YEAR;
    let actual = simulate_update_indexes(&env, two_years, &sync);

    let (expected_borrow_index, expected_supply_index) = oracle_accrual(
        &env,
        &params,
        Ray::from(60 * RAY),
        Ray::ZERO,
        Ray::ONE,
        Ray::ONE,
        &[MAX_COMPOUND_DELTA_MS, MAX_COMPOUND_DELTA_MS],
    );

    assert_eq!(actual.borrow_index.raw(), expected_borrow_index.raw());
    assert_eq!(actual.supply_index.raw(), expected_supply_index.raw());
}
