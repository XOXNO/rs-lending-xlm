use super::*;
use crate::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
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
fn test_supply_index_shortfall_accounts_full_reward() {
    let env = Env::default();
    // Funded market: 1,000 tokens (7dp) supplied at index RAY, 100-token reward.
    let supplied = Ray::from_asset(1_000, 7);
    let old_index = Ray::from(RAY);
    let reward = Ray::from_asset(100, 7);

    let new_index = update_supply_index(&env, supplied, old_index, reward);
    let shortfall = supply_index_reward_shortfall(&env, supplied, old_index, new_index, reward);
    let distributed = supplied
        .mul(&env, new_index)
        .checked_sub(&env, supplied.mul(&env, old_index));

    // 100% accounted: suppliers (via index) + protocol (shortfall) == full reward.
    assert_eq!(
        distributed.checked_add(&env, shortfall),
        reward,
        "distributed + shortfall must equal the full reward (no dead reserve)"
    );
    // The virtual offset genuinely under-distributes, so the shortfall is positive
    // and suppliers keep only their diluted (dust-safe) share.
    assert!(
        shortfall.raw() > 0,
        "offset must leave a positive shortfall"
    );
    assert!(
        distributed.raw() > 0 && distributed.raw() < reward.raw(),
        "suppliers receive the diluted share, strictly less than the full reward"
    );
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
    // Growth = rewards / (supplied_value + virtual offset) = 5 / 101.
    let expected = RAY * 106 / 101;
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

// Borrow index at MAX returns MAX; above MAX clamps (not panic).

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
    // factor = 1 + 1 ulp → product > MAX → clamp.
    let factor = Ray::from(RAY + 1);
    let new_index = update_borrow_index(&env, old_index, factor);
    assert_eq!(new_index.raw(), MAX_BORROW_INDEX_RAY);
}

// Nonzero delta + live debt must accrue (not a no-op).
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

// Multi-year deltas use 1y chunks; chunked compound > single long Taylor eval.
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

// Split accrual cannot lower indexes vs a single shot (no borrow-time farming).
#[test]
fn test_split_accrual_never_reduces_borrow_index() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    let asset = soroban_sdk::Address::from_str(
        &env,
        "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
    );
    let mk = |borrow_index: i128, supply_index: i128, last_timestamp: u64| PoolSyncData {
        params: MarketParamsRaw {
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
            asset_id: asset.clone(),
            asset_decimals: 7,
        },
        state: PoolStateRaw {
            supplied: 100 * RAY,
            borrowed: 80 * RAY,
            revenue: 0,
            borrow_index,
            supply_index,
            last_timestamp,
            cash: 20_000_000,
        },
    };

    // Sub-chunk interval so the single call is exactly one Taylor evaluation.
    let total = MAX_COMPOUND_DELTA_MS / 2;
    let single = simulate_update_indexes(&env, total, &mk(RAY, RAY, 0));

    // Same interval, split at an arbitrary interior point (two Taylor evals).
    let split_at = total * 3 / 7;
    let step1 = simulate_update_indexes(&env, split_at, &mk(RAY, RAY, 0));
    let split = simulate_update_indexes(
        &env,
        total,
        &mk(step1.borrow_index.raw(), step1.supply_index.raw(), split_at),
    );

    assert!(
        split.borrow_index.raw() >= single.borrow_index.raw(),
        "split must not lower borrow index: split={} single={}",
        split.borrow_index.raw(),
        single.borrow_index.raw()
    );
    assert!(
        split.supply_index.raw() >= single.supply_index.raw(),
        "split must not lower supply index: split={} single={}",
        split.supply_index.raw(),
        single.supply_index.raw()
    );
    // Cadence alone must not double the index (loose runaway guard).
    assert!(
        split.borrow_index.raw() <= single.borrow_index.raw() * 2,
        "split ran away vs single: split={} single={}",
        split.borrow_index.raw(),
        single.borrow_index.raw()
    );
}

// compound_interest ≈ e^0.5; tolerance catches Taylor term sign flips.
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

// At util == mid: correct path adds 0 slope1; wrong `<=` leaks half-up drift.
#[test]
fn test_calculate_borrow_rate_mid_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params();
    params.mid_utilization = Ray::from(RAY / 3);
    params.slope1 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization.checked_add(&env, Ray::from(RAY / 5));

    let rate = calculate_borrow_rate(&env, params.mid_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == mid_utilization must take the slope2 branch (zero contribution)"
    );
}

// At util == optimal: correct path adds 0 slope2; wrong `<=` leaks drift.
#[test]
fn test_calculate_borrow_rate_optimal_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params();
    params.mid_utilization = Ray::from(RAY / 5);
    params.slope1 = Ray::ZERO;
    params.slope2 = Ray::from(186_742_236_914_318_803_376_138_999_i128);
    params.optimal_utilization = params.mid_utilization.checked_add(&env, Ray::from(RAY / 3));

    let rate = calculate_borrow_rate(&env, params.optimal_utilization, &params);

    assert_eq!(
        rate.raw(),
        6_234_518_435_487_626_i128,
        "utilization == optimal_utilization must take the slope3 branch (zero contribution)"
    );
}

// Public-primitive oracle for per-chunk accrual + fee reinvestment guards.
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

        // Fee reinvestment mirrors the live path.
        if protocol_fee != Ray::ZERO {
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
fn test_simulate_matches_mirror_at_supply_index_floor() {
    use crate::types::{MarketParamsRaw, PoolStateRaw, PoolSyncData};

    let env = Env::default();
    // 100% reserve factor: no supplier rewards, supply_index stays at the floor;
    // all interest is fee and both paths reinvest it identically.
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
fn test_simulate_matches_mirror_when_supplied_zero() {
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

#[test]
fn test_deposit_rate_reserve_factor_out_of_range_returns_zero() {
    let env = Env::default();
    // reserve_factor == BPS is outside [0, BPS); supplier rate collapses to zero.
    let rate = calculate_deposit_rate(
        &env,
        Ray::from(RAY / 2),
        Ray::from(RAY / 10),
        Bps::from(crate::constants::BPS),
    );
    assert_eq!(rate, Ray::ZERO);
}

#[test]
fn test_update_supply_index_rounds_supplied_value_to_zero_returns_old_index() {
    let env = Env::default();
    // 1 * 1 / RAY == 0, so total_supplied_value is zero despite nonzero rewards.
    let out = update_supply_index(&env, Ray::from(1), Ray::from(1), Ray::from(5 * RAY));
    assert_eq!(out, Ray::from(1));
}

#[test]
fn test_simulate_update_indexes_zero_delta_is_noop() {
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
        borrow_index: 2 * RAY,
        supply_index: 3 * RAY,
        last_timestamp: 1_000,
        cash: 40_000_000,
    };
    let sync = PoolSyncData { params, state };
    // Query at the checkpoint timestamp: delta == 0 returns the stored indexes verbatim.
    let indexes = simulate_update_indexes(&env, 1_000, &sync);
    assert_eq!(indexes.borrow_index, Ray::from(2 * RAY));
    assert_eq!(indexes.supply_index, Ray::from(3 * RAY));
}

// --- POOL-CAN-001: virtual offset bounds dust-reward growth. ---

/// Dust supply + large reward: index grows but stays below the cap.
#[test]
fn test_virtual_offset_bounds_dust_reward_growth() {
    let env = Env::default();

    let clamped = update_borrow_index(&env, Ray::from(MAX_BORROW_INDEX_RAY), Ray::from(RAY * 2));
    assert_eq!(clamped.raw(), MAX_BORROW_INDEX_RAY);

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from_asset(170_141_183_459, 7);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert!(grown.raw() > RAY, "reward must still grow the index");
    assert!(
        grown.raw() < MAX_SUPPLY_INDEX_RAY,
        "offset must keep growth below the cap"
    );
    assert!(
        grown.raw() < RAY * 1_000_000,
        "growth is bounded to ~1.7e31"
    );
}

/// Bounded index still accepts a later ordinary accrual.
#[test]
fn test_offset_supply_index_survives_ordinary_accrual() {
    let env = Env::default();

    let grown = update_supply_index(
        &env,
        Ray::from_asset(1, 7),
        Ray::from(RAY),
        Ray::from_asset(170_141_183_459, 7),
    );
    assert!(grown.raw() < MAX_SUPPLY_INDEX_RAY);

    let next = update_supply_index(&env, Ray::from(1), grown, Ray::from(170_000));
    assert!(next.raw() >= grown.raw());
    assert!(next.raw() < MAX_SUPPLY_INDEX_RAY);
}

/// Extreme reward still clamps at `MAX_SUPPLY_INDEX_RAY`.
#[test]
fn test_cap_still_backstops_extreme_reward() {
    let env = Env::default();

    let supplied = Ray::from_asset(1, 7);
    let reward = Ray::from(i128::MAX / 2);

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), reward);

    assert_eq!(grown.raw(), MAX_SUPPLY_INDEX_RAY);
}

/// Funded market: offset dilutes growth by less than 1%.
#[test]
fn test_virtual_offset_negligible_for_funded_market() {
    let env = Env::default();
    let supplied = Ray::from(1_000 * RAY); // 1000 tokens
    let rewards = Ray::from(10 * RAY); // 1% reward

    let grown = update_supply_index(&env, supplied, Ray::from(RAY), rewards);

    // 1 + 10/1001 with offset; 1 + 10/1000 without.
    let with_offset = RAY + RAY * 10 / 1001;
    let offset_free = RAY + RAY * 10 / 1000;
    assert!((grown.raw() - with_offset).abs() <= 1);

    let drift = offset_free - grown.raw();
    assert!(
        drift * 100 < offset_free - RAY,
        "dilution < 1% of reward growth"
    );
}

#[test]
fn protocol_fee_shares_matches_half_up_divide_in_range() {
    let env = Env::default();
    let supply_index = Ray::from(2 * RAY);
    let fee = Ray::from(500 * RAY);
    let supplied = Ray::from(1_000_000 * RAY);
    // In-range results are byte-identical to the plain half-up `fee / supply_index`.
    assert_eq!(
        protocol_fee_shares(&env, fee, supply_index, supplied).raw(),
        fee.div(&env, supply_index).raw(),
    );
}

#[test]
fn protocol_fee_shares_saturates_and_caps_at_floored_index() {
    let env = Env::default();
    // Post-wipeout floored index: the plain divide would push the share count past
    // i128 and trap. The overflow-safe form saturates, then caps to supply headroom.
    let supply_index = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let fee = Ray::from(i128::MAX / 100);
    let supplied = Ray::from(1_000 * RAY);
    let shares = protocol_fee_shares(&env, fee, supply_index, supplied);
    assert_eq!(shares.raw(), i128::MAX - supplied.raw());
}

// --- AUDIT: Controller::add_rewards iterated-leg supply-index pinning ---

/// Proof for the surviving hypothesis: the single-shot virtual-offset defense in
/// `update_supply_index` does NOT bound growth when the SAME dust market is fed
/// many reward legs in sequence (as `Controller::add_rewards` does, one
/// load->update->save per non-deduplicated Vec leg). Each leg reloads the
/// persisted index, so growth COMPOUNDS across legs. With a 1-raw-unit seed on a
/// 7-decimal asset, ~30 modest legs (each ~doubling the index) drive `supply_index`
/// to the sticky `MAX_SUPPLY_INDEX_RAY` clamp for a modest total reward outlay,
/// after which ALL supplier yield is permanently discarded.
#[test]
fn audit_controller_add_rewards_iterated_legs_pin_supply_index_and_zero_yield() {
    let env = Env::default();

    // Attacker seeds 1 raw unit of a 7-decimal asset (dust: 1e-7 tokens of value).
    let supplied = Ray::from_asset(1, 7);
    let mut index = Ray::from(RAY);

    // Walk the market by feeding legs that each roughly DOUBLE the index: reward =
    // (total_supplied_value + virtual_offset), i.e. exactly the reward denominator,
    // so factor = 1 + denom/denom = 2. This is the small-step regime the offset was
    // meant to bound; iterated it compounds geometrically.
    let mut total_reward_raw: i128 = 0;
    let mut legs = 0u32;
    while index.raw() < MAX_SUPPLY_INDEX_RAY && legs < 40 {
        let tsv = supplied.mul(&env, index).raw();
        let reward_raw = tsv + SUPPLY_VIRTUAL_VALUE_RAY; // == denom -> factor 2
        total_reward_raw = total_reward_raw.saturating_add(reward_raw);
        index = update_supply_index(&env, supplied, index, Ray::from(reward_raw));
        legs += 1;
    }

    // EXPLOIT ASSERTION 1: iterated legs pin the index at the sticky clamp.
    assert_eq!(
        index.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "iterated add_rewards legs must pin supply_index at MAX",
    );
    assert!(legs <= 31, "cap reached in ~30 modest legs, got {legs}");

    // Total reward outlay stays modest (~hundreds of whole tokens on a 7-dp asset):
    // final leg cost ~= offset-in-tokens at the cap (~100 tokens), most recoverable
    // by the sole supplier's own withdraw. Net cost is a small stranded remainder.
    let total_reward_tokens = total_reward_raw / RAY; // whole tokens
    assert!(
        total_reward_tokens < 1_000,
        "total reward outlay to pin the market is modest ({total_reward_tokens} tokens)",
    );

    // EXPLOIT ASSERTION 2: with the index pinned, an ordinary later supplier-reward
    // accrual (real borrow interest, sized as tokens) is silently DISCARDED — the
    // clamp re-applies and the index does not move. Supplier yield is 0% forever.
    let ordinary_reward = Ray::from_asset(1_000, 7); // 1000 tokens of real interest
    let after = update_supply_index(&env, supplied, index, ordinary_reward);
    assert_eq!(
        after.raw(),
        MAX_SUPPLY_INDEX_RAY,
        "post-pin, real supplier interest is clamped away: index unchanged (0% yield)",
    );
}
