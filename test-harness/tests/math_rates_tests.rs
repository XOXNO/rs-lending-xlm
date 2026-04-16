extern crate std;

use common::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use common::fp::Ray;
use common::fp_core::{div_by_int_half_up, mul_div_half_up, mul_div_half_up_signed, rescale_half_up};
use common::rates::*;
use soroban_sdk::Env;

// ===========================================================================
// Math edge cases
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. test_rescale_same_decimals
// ---------------------------------------------------------------------------

#[test]
fn test_rescale_same_decimals() {
    assert_eq!(rescale_half_up(12345, 7, 7), 12345);
    assert_eq!(rescale_half_up(0, 18, 18), 0);
    assert_eq!(rescale_half_up(-100, 6, 6), -100);
}

// ---------------------------------------------------------------------------
// 2. test_rescale_upscale
// ---------------------------------------------------------------------------

#[test]
fn test_rescale_upscale() {
    // 100 at 7 decimals -> 18 decimals = 100 * 10^11
    let result = rescale_half_up(100, 7, 18);
    assert_eq!(result, 100 * 100_000_000_000i128);
}

// ---------------------------------------------------------------------------
// 3. test_rescale_downscale_half_up
// ---------------------------------------------------------------------------

#[test]
fn test_rescale_downscale_half_up() {
    let result = rescale_half_up(1_500_000_000_000i128, 18, 7);
    assert_eq!(result, 15);

    let result = rescale_half_up(1_550_000_000_000i128, 18, 7);
    assert_eq!(result, 16);

    let result = rescale_half_up(1_449_999_999_999i128, 18, 7);
    assert_eq!(result, 14);
}

// ---------------------------------------------------------------------------
// 4. test_mul_div_half_up_zero
// ---------------------------------------------------------------------------

#[test]
fn test_mul_div_half_up_zero() {
    let env = Env::default();
    assert_eq!(mul_div_half_up(&env, 0, RAY, RAY), 0);
    assert_eq!(mul_div_half_up(&env, RAY, 0, RAY), 0);
    assert_eq!(mul_div_half_up(&env, 0, 0, RAY), 0);
}

// ---------------------------------------------------------------------------
// 5. test_mul_div_half_up_precision_boundary
// ---------------------------------------------------------------------------

#[test]
fn test_mul_div_half_up_precision_boundary() {
    let env = Env::default();
    // 3 * (WAD/2) / WAD: 3 * 0.5 = 1.5, rounds up to 2
    let result = mul_div_half_up(&env, 3, WAD / 2, WAD);
    assert_eq!(result, 2);

    // 1 * (WAD/2) / WAD: 0.5, rounds up to 1
    let result = mul_div_half_up(&env, 1, WAD / 2, WAD);
    assert_eq!(result, 1);
}

// ---------------------------------------------------------------------------
// 6. test_div_half_up_exact
// ---------------------------------------------------------------------------

#[test]
fn test_div_half_up_exact() {
    let env = Env::default();
    // 10 / 2 = 5 exactly (no rounding needed)
    let result = mul_div_half_up(&env, 10 * WAD, WAD, 2 * WAD);
    assert_eq!(result, 5 * WAD);
}

// ---------------------------------------------------------------------------
// 7. test_div_half_up_rounds_up
// ---------------------------------------------------------------------------

#[test]
fn test_div_half_up_rounds_up() {
    let env = Env::default();
    // 2/3 in WAD: result is 0.666... which rounds up
    let result = mul_div_half_up(&env, 2 * WAD, WAD, 3 * WAD);
    assert_eq!(result, 666_666_666_666_666_667);
}

// ---------------------------------------------------------------------------
// 8. test_mul_half_up_signed_negative
// ---------------------------------------------------------------------------

#[test]
fn test_mul_half_up_signed_negative() {
    let env = Env::default();
    // -3 * 0.5 = -1.5, rounds away from zero to -2
    let result = mul_div_half_up_signed(&env, -3, WAD / 2, WAD);
    assert_eq!(result, -2);

    // -1 * 0.5 = -0.5, rounds away from zero to -1
    let result = mul_div_half_up_signed(&env, -1, WAD / 2, WAD);
    assert_eq!(result, -1);
}

// ---------------------------------------------------------------------------
// 9. test_div_half_up_signed_negative
// ---------------------------------------------------------------------------

#[test]
fn test_div_half_up_signed_negative() {
    let env = Env::default();
    // For signed division, we test via the signed primitive
    // -2/3: product = -2*WAD*WAD, negative so subtract half => rounds away from zero
    // We compute: mul_div_half_up_signed(-2*WAD, WAD, 3*WAD) which is signed div
    let result = mul_div_half_up_signed(&env, -2 * WAD, WAD, 3 * WAD);
    assert_eq!(result, -666_666_666_666_666_667);

    let result = mul_div_half_up_signed(&env, -WAD, WAD, 3 * WAD);
    assert_eq!(result, -333_333_333_333_333_333);
}

// ---------------------------------------------------------------------------
// 10. test_div_by_int_half_up
// ---------------------------------------------------------------------------

#[test]
fn test_div_by_int_half_up() {
    // 7 / 2 = 3.5, rounds up to 4
    assert_eq!(div_by_int_half_up(7, 2), 4);
    // 6 / 2 = 3 exactly
    assert_eq!(div_by_int_half_up(6, 2), 3);
    // 5 / 3 = 1.666..., half_b=1, (5+1)/3 = 2
    assert_eq!(div_by_int_half_up(5, 3), 2);
}

// ---------------------------------------------------------------------------
// 11. test_min_max_equal
// ---------------------------------------------------------------------------

#[test]
fn test_min_max_equal() {
    assert_eq!(5i128.min(5), 5);
    assert_eq!(5i128.max(5), 5);
    assert_eq!((-3i128).min(-3), -3);
    assert_eq!((-3i128).max(-3), -3);
}

// ===========================================================================
// Rates edge cases
// ===========================================================================

fn make_test_params() -> common::types::MarketParams {
    common::types::MarketParams {
        base_borrow_rate_ray: RAY / 100,         // 1%
        slope1_ray: RAY * 4 / 100,               // 4%
        slope2_ray: RAY * 10 / 100,              // 10%
        slope3_ray: RAY * 300 / 100,             // 300%
        mid_utilization_ray: RAY * 50 / 100,     // 50%
        optimal_utilization_ray: RAY * 80 / 100, // 80%
        max_borrow_rate_ray: RAY,                // 100%
        reserve_factor_bps: 1000,                // 10%
        asset_id: soroban_sdk::Address::from_str(
            &Env::default(),
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
        ),
        asset_decimals: 7,
    }
}

// ---------------------------------------------------------------------------
// 12. test_borrow_rate_zero_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rate_zero_utilization() {
    let env = Env::default();
    let params = make_test_params();

    let rate = calculate_borrow_rate(&env, Ray::ZERO, &params);
    let expected = div_by_int_half_up(RAY / 100, MILLISECONDS_PER_YEAR as i128);
    assert_eq!(rate.raw(), expected);
}

// ---------------------------------------------------------------------------
// 13. test_borrow_rate_at_mid_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rate_at_mid_utilization() {
    let env = Env::default();
    let params = make_test_params();

    let util_mid = RAY * 50 / 100;
    let rate = calculate_borrow_rate(&env, Ray::from_raw(util_mid), &params);
    let expected_annual = RAY * 5 / 100;
    let expected = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 14. test_borrow_rate_at_optimal_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rate_at_optimal_utilization() {
    let env = Env::default();
    let params = make_test_params();

    let util_opt = RAY * 80 / 100;
    let rate = calculate_borrow_rate(&env, Ray::from_raw(util_opt), &params);
    let expected_annual = RAY * 15 / 100;
    let expected = div_by_int_half_up(expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 15. test_borrow_rate_full_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rate_full_utilization() {
    let env = Env::default();
    let params = make_test_params();

    let rate = calculate_borrow_rate(&env, Ray::ONE, &params);
    let expected = div_by_int_half_up(params.max_borrow_rate_ray, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 16. test_borrow_rate_capped_at_max
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_rate_capped_at_max() {
    let env = Env::default();
    let params = make_test_params();

    let rate = calculate_borrow_rate(&env, Ray::from_raw(RAY * 90 / 100), &params);
    let max_rate = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - max_rate).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 17. test_deposit_rate_zero_utilization
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_rate_zero_utilization() {
    let env = Env::default();
    let rate = calculate_deposit_rate(&env, Ray::ZERO, Ray::from_raw(RAY / 10), 1000);
    assert_eq!(rate, Ray::ZERO);
}

// ---------------------------------------------------------------------------
// 18. test_deposit_rate_with_reserve_factor
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_rate_with_reserve_factor() {
    let env = Env::default();
    let rate = calculate_deposit_rate(
        &env,
        Ray::from_raw(RAY * 80 / 100),
        Ray::from_raw(RAY * 5 / 100),
        1000,
    );
    let expected = RAY * 36 / 1000;
    assert!((rate.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 19. test_compound_interest_zero_delta
// ---------------------------------------------------------------------------

#[test]
fn test_compound_interest_zero_delta() {
    let env = Env::default();
    let result = compound_interest(&env, Ray::from_raw(RAY / 10), 0);
    assert_eq!(result, Ray::ONE);
}

// ---------------------------------------------------------------------------
// 20. test_compound_interest_one_year
// ---------------------------------------------------------------------------

#[test]
fn test_compound_interest_one_year() {
    let env = Env::default();

    let annual_rate = RAY / 10;
    let rate_per_ms = div_by_int_half_up(annual_rate, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&env, Ray::from_raw(rate_per_ms), MILLISECONDS_PER_YEAR);

    let expected = 1_105_170_918_075_647_624_811_707_826_i128;

    let diff = (factor.raw() - expected).abs();
    let tolerance = expected / 1_000_000;
    assert!(
        diff < tolerance,
        "compound interest accuracy: got {}, expected {}, diff={}",
        factor.raw(),
        expected,
        diff
    );
}

// ---------------------------------------------------------------------------
// 21. test_utilization_zero_supply
// ---------------------------------------------------------------------------

#[test]
fn test_utilization_zero_supply() {
    let env = Env::default();
    assert_eq!(utilization(&env, Ray::from_raw(50 * RAY), Ray::ZERO), Ray::ZERO);
}

// ---------------------------------------------------------------------------
// 22. test_utilization_over_one
// ---------------------------------------------------------------------------

#[test]
fn test_utilization_over_one() {
    let env = Env::default();
    let util = utilization(&env, Ray::from_raw(150 * RAY), Ray::from_raw(100 * RAY));
    let expected = RAY * 3 / 2;
    assert!((util.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 23. test_supply_index_update_zero_rewards
// ---------------------------------------------------------------------------

#[test]
fn test_supply_index_update_zero_rewards() {
    let env = Env::default();
    let result = update_supply_index(&env, Ray::from_raw(100 * RAY), Ray::ONE, Ray::ZERO);
    assert_eq!(result, Ray::ONE, "zero rewards should leave index unchanged");
}

// ---------------------------------------------------------------------------
// 24. test_supply_index_update_with_rewards
// ---------------------------------------------------------------------------

#[test]
fn test_supply_index_update_with_rewards() {
    let env = Env::default();
    let new_index = update_supply_index(&env, Ray::from_raw(100 * RAY), Ray::ONE, Ray::from_raw(5 * RAY));
    let expected = RAY * 105 / 100;
    assert!((new_index.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 25. test_borrow_index_update
// ---------------------------------------------------------------------------

#[test]
fn test_borrow_index_update() {
    let env = Env::default();
    let factor = RAY + RAY * 5 / 100;
    let new_index = update_borrow_index(&env, Ray::ONE, Ray::from_raw(factor));
    let expected = RAY * 105 / 100;
    assert!((new_index.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 26. test_supplier_rewards_split
// ---------------------------------------------------------------------------

#[test]
fn test_supplier_rewards_split() {
    let env = Env::default();
    let params = make_test_params();

    let borrowed = Ray::from_raw(100 * RAY);
    let old_index = Ray::ONE;
    let new_index = Ray::from_raw(RAY + RAY / 100);

    let (rewards, fee) = calculate_supplier_rewards(&env, &params, borrowed, new_index, old_index);

    let expected_fee = RAY / 10;
    let expected_rewards = RAY * 9 / 10;

    assert!((fee.raw() - expected_fee).abs() <= 1);
    assert!((rewards.raw() - expected_rewards).abs() <= 1);
    assert_eq!(
        rewards.raw() + fee.raw(),
        RAY,
        "rewards + fee should equal total interest"
    );
}

// ---------------------------------------------------------------------------
// 27. test_scaled_to_original
// ---------------------------------------------------------------------------

#[test]
fn test_scaled_to_original_basic() {
    let env = Env::default();
    let result = scaled_to_original(&env, Ray::from_raw(100 * RAY), Ray::from_raw(RAY * 105 / 100));
    let expected = 105 * RAY;
    assert!((result.raw() - expected).abs() <= 1);
}

// ---------------------------------------------------------------------------
// 28. test_compound_interest_small_rate
// ---------------------------------------------------------------------------

#[test]
fn test_compound_interest_small_rate() {
    let env = Env::default();
    let annual_rate = RAY / 10_000;
    let rate_per_ms = div_by_int_half_up(annual_rate, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&env, Ray::from_raw(rate_per_ms), MILLISECONDS_PER_YEAR);

    assert!(
        factor.raw() > RAY,
        "compound factor should be > 1.0 for positive rate"
    );
    let growth = factor.raw() - RAY;
    assert!(
        growth > RAY / 10_001 && growth < RAY / 9_999,
        "growth {} should be ~{}",
        growth,
        RAY / 10_000
    );
}
