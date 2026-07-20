//! Edge-case and overflow probes at protocol decision boundaries.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BAD_DEBT_USD_THRESHOLD, MILLISECONDS_PER_YEAR, RAY, WAD};
use crate::types::MarketParams;
use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core::{div_by_int_half_up, mul_div_half_up, rescale_half_up};
use common::rates::{calculate_borrow_rate, compound_interest};

/// Fixed params with known utilization breakpoints (1/4/10/80% slopes, 50/80/95% util).
fn boundary_test_params(env: &Env) -> MarketParams {
    MarketParams {
        base_borrow_rate: Ray::from(RAY / 100),
        slope1: Ray::from(RAY * 4 / 100),
        slope2: Ray::from(RAY * 10 / 100),
        slope3: Ray::from(RAY * 80 / 100),
        mid_utilization: Ray::from(RAY * 50 / 100),
        optimal_utilization: Ray::from(RAY * 80 / 100),
        max_utilization: Ray::from(RAY * 95 / 100),
        max_borrow_rate: Ray::from(RAY),
        reserve_factor: Bps::from(1000),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: env.current_contract_address(),
        asset_decimals: 7,
    }
}

#[rule]
fn borrow_rate_at_exact_zero_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::ZERO, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

#[rule]
fn borrow_rate_at_exact_mid_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, params.mid_utilization, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

#[rule]
fn borrow_rate_at_exact_optimal_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, params.optimal_utilization, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

#[rule]
fn borrow_rate_at_100_percent_sanity(e: Env) {
    let params = boundary_test_params(&e);
    let rate = calculate_borrow_rate(&e, Ray::ONE, &params);
    cvlr_satisfy!(rate.raw() > 0);
}

#[rule]
fn compound_interest_at_max_rate_max_time_sanity(e: Env) {
    let rate_per_ms = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    cvlr_satisfy!(factor.raw() > 2 * RAY && factor.raw() < 3 * RAY);
}

/// Production `is_socializable_bad_debt` boundary: underwater collateral at
/// exactly the threshold socializes; one unit above never does; accounts that
/// are not underwater never do.
#[rule]
fn bad_debt_socialization_threshold_boundary(e: Env, debt_wad: i128, collateral_wad: i128) {
    let _ = e;
    cvlr_assume!(debt_wad > 0 && debt_wad <= 1_000_000 * WAD);
    cvlr_assume!(collateral_wad >= 0 && collateral_wad <= 1_000_000 * WAD);

    let socializable = crate::positions::liquidation::math::is_socializable_bad_debt(
        Wad::from(debt_wad),
        Wad::from(collateral_wad),
    );

    if collateral_wad > BAD_DEBT_USD_THRESHOLD {
        cvlr_assert!(!socializable);
    }
    if debt_wad <= collateral_wad {
        cvlr_assert!(!socializable);
    }
    if debt_wad > collateral_wad && collateral_wad <= BAD_DEBT_USD_THRESHOLD {
        cvlr_assert!(socializable);
    }
}

/// `mul_half_up(i128::MAX / RAY, RAY)` does not overflow via I256 intermediate.
#[rule]
fn mul_at_max_i128(e: Env) {
    let a = i128::MAX / RAY;
    let result = mul_div_half_up(&e, a, RAY, RAY);
    cvlr_assert!(result >= a - 1 && result <= a + 1);
}

#[rule]
fn mul_at_max_i128_sanity(e: Env) {
    let a = i128::MAX / RAY;
    let result = mul_div_half_up(&e, a, RAY, RAY);
    cvlr_satisfy!(result > 0);
}

#[rule]
fn compound_taylor_accuracy(e: Env) {
    let annual_rate_ray = RAY / 100;
    let rate_per_ms = div_by_int_half_up(annual_rate_ray, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    let tolerance = RAY / 10_000;
    let lower = RAY + annual_rate_ray;

    cvlr_assert!(factor.raw() > RAY);
    cvlr_assert!(factor.raw() >= lower);
    cvlr_assert!(factor.raw() < lower + tolerance);
}

#[rule]
fn compound_taylor_accuracy_sanity(e: Env) {
    let rate_per_ms = div_by_int_half_up(RAY / 100, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    cvlr_satisfy!(factor.raw() > RAY + RAY / 100);
}

#[rule]
fn rescale_ray_to_wad() {
    let result = rescale_half_up(RAY, 27, 18);
    cvlr_assert!(result == WAD);
}

#[rule]
fn rescale_wad_to_7_decimals() {
    let result = rescale_half_up(WAD, 18, 7);
    cvlr_assert!(result == 10_000_000i128);
}

#[rule]
fn supply_dust_amount_sanity(e: Env) {
    let scaled = mul_div_half_up(&e, 1, RAY, RAY);
    cvlr_satisfy!(scaled == 1);
}
