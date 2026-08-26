use super::*;
use crate::constants::{MILLISECONDS_PER_YEAR, RAY};
use crate::math::fp_core::div_by_int_half_up;
use crate::rates::test_support::*;
use soroban_sdk::Env;

#[test]
fn test_borrow_rate_region1() {
    let env = Env::default();
    let params = make_test_params(&env);

    let rate = calculate_borrow_rate(&env, Ray::ZERO, &params);
    let expected = div_by_int_half_up(&env, RAY / 100, MILLISECONDS_PER_YEAR as i128);
    assert_eq!(rate.raw(), expected);

    let util_25 = Ray::from(RAY * 25 / 100);
    let rate = calculate_borrow_rate(&env, util_25, &params);
    let expected_annual = RAY * 3 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region2() {
    let env = Env::default();
    let params = make_test_params(&env);

    let util_50 = Ray::from(RAY * 50 / 100);
    let rate = calculate_borrow_rate(&env, util_50, &params);
    let expected_annual = RAY * 5 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_65 = Ray::from(RAY * 65 / 100);
    let rate = calculate_borrow_rate(&env, util_65, &params);
    let expected_annual = RAY * 10 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_region3() {
    let env = Env::default();
    let params = make_test_params(&env);

    let util_80 = Ray::from(RAY * 80 / 100);
    let rate = calculate_borrow_rate(&env, util_80, &params);
    let expected_annual = RAY * 15 / 100;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);

    let util_90 = Ray::from(RAY * 90 / 100);
    let rate = calculate_borrow_rate(&env, util_90, &params);
    let expected_annual = RAY;
    let expected_per_ms = div_by_int_half_up(&env, expected_annual, MILLISECONDS_PER_YEAR as i128);
    assert!((rate.raw() - expected_per_ms).abs() <= 1);
}

#[test]
fn test_borrow_rate_capped() {
    let env = Env::default();
    let params = make_test_params(&env);

    let rate = calculate_borrow_rate(&env, Ray::ONE, &params);
    let expected = div_by_int_half_up(
        &env,
        params.max_borrow_rate.raw(),
        MILLISECONDS_PER_YEAR as i128,
    );
    assert!((rate.raw() - expected).abs() <= 1);
}

#[test]
fn test_borrow_rate_clamps_utilization_above_one() {
    let env = Env::default();

    let mut raw = make_test_params_raw(&env);
    raw.optimal_utilization = RAY - 1;
    raw.max_utilization = RAY;
    let params = MarketParams::from(&raw);

    let rate = calculate_borrow_rate(&env, Ray::from(RAY * 2), &params);
    let expected = div_by_int_half_up(
        &env,
        params.max_borrow_rate.raw(),
        MILLISECONDS_PER_YEAR as i128,
    );
    assert!(rate.raw() > 0);
    assert!(
        rate.raw() <= expected + 1,
        "util > RAY must clamp and stay bounded by the max-rate cap"
    );
}

#[test]
fn test_calculate_borrow_rate_mid_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params(&env);
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

#[test]
fn test_calculate_borrow_rate_optimal_utilization_boundary_exact() {
    let env = Env::default();
    let mut params = make_test_params(&env);
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

#[test]
fn test_annual_borrow_rate_matches_curve_before_year_conversion() {
    let env = Env::default();
    let params = make_test_params(&env);

    assert_eq!(
        calculate_annual_borrow_rate(&env, Ray::ZERO, &params).raw(),
        RAY / 100
    );

    let util_50 = Ray::from(RAY * 50 / 100);
    assert_eq!(
        calculate_annual_borrow_rate(&env, util_50, &params).raw(),
        RAY * 5 / 100
    );
}

#[test]
fn test_borrow_rate_is_annual_divided_by_milliseconds_per_year() {
    let env = Env::default();
    let params = make_test_params(&env);
    let util = Ray::from(RAY * 65 / 100);
    let annual = calculate_annual_borrow_rate(&env, util, &params);
    let per_ms = calculate_borrow_rate(&env, util, &params);
    assert_eq!(
        per_ms.raw(),
        annual.div_by_int(&env, MILLISECONDS_PER_YEAR as i128).raw()
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

#[test]
fn test_deposit_rate_reserve_factor_out_of_range_returns_zero() {
    let env = Env::default();

    let rate = calculate_deposit_rate(
        &env,
        Ray::from(RAY / 2),
        Ray::from(RAY / 10),
        Bps::from(crate::constants::BPS),
    );
    assert_eq!(rate, Ray::ZERO);
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
