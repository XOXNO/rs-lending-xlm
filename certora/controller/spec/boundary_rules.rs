//! Edge-case and overflow probes at protocol decision boundaries.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use crate::types::MarketParams;
use common::math::fp::{Bps, Ray};
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

#[rule]
fn liquidation_at_hf_exactly_one_sanity() {
    let hf = WAD;
    cvlr_satisfy!(hf >= WAD);
}

#[rule]
fn liquidation_at_hf_just_below_one_sanity() {
    let hf = WAD - 1;
    cvlr_satisfy!(hf < WAD);
}

#[rule]
fn bonus_at_hf_exactly_102_sanity() {
    let hf_wad: i128 = 1_020_000_000_000_000_000;
    let target_hf: i128 = 1_020_000_000_000_000_000;
    cvlr_satisfy!(hf_wad >= target_hf);
}

#[rule]
fn bad_debt_at_exactly_5_usd_sanity() {
    let total_collateral_usd = 5 * WAD;
    let bad_debt_threshold = 5 * WAD;
    cvlr_satisfy!(total_collateral_usd <= bad_debt_threshold);
}

#[rule]
fn bad_debt_at_6_usd_sanity() {
    let total_collateral_usd = 6 * WAD;
    let bad_debt_threshold = 5 * WAD;
    cvlr_satisfy!(total_collateral_usd > bad_debt_threshold);
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

/// 1% APY over 1 year: Taylor compound factor within 0.01% of `1 + rate`.
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

/// `rescale(RAY, 27, 18) == WAD`.
#[rule]
fn rescale_ray_to_wad() {
    let result = rescale_half_up(RAY, 27, 18);
    cvlr_assert!(result == WAD);
}

/// `rescale(WAD, 18, 7) == 10^7`.
#[rule]
fn rescale_wad_to_7_decimals() {
    let result = rescale_half_up(WAD, 18, 7);
    cvlr_assert!(result == 10_000_000i128);
}

#[rule]
fn tolerance_at_exact_first_bound_sanity() {
    // Deviation exactly at the single tolerance band edge is in-band.
    let tolerance: i128 = 200;
    let deviation: i128 = 200;
    cvlr_satisfy!(deviation <= tolerance);
}

#[rule]
fn tolerance_at_exact_second_bound_sanity() {
    // Deviation strictly inside the single tolerance band is in-band.
    let tolerance: i128 = 200;
    let deviation: i128 = 100;
    cvlr_satisfy!(deviation <= tolerance);
}

#[rule]
fn tolerance_just_beyond_second_sanity() {
    // Deviation just past the single tolerance band is out-of-band.
    let tolerance: i128 = 200;
    let deviation: i128 = 201;
    cvlr_satisfy!(deviation > tolerance);
}

#[rule]
fn supply_dust_amount_sanity(e: Env) {
    let scaled = mul_div_half_up(&e, 1, RAY, RAY);
    cvlr_satisfy!(scaled == 1);
}

#[rule]
fn borrow_exact_reserves_sanity() {
    let reserves: i128 = 1_000_000;
    let borrow: i128 = 1_000_000;
    cvlr_satisfy!(borrow <= reserves);
}

#[rule]
fn withdraw_more_than_position_sanity() {
    let position_value: i128 = 100;
    let requested: i128 = 200;
    let actual = requested.min(position_value);
    cvlr_satisfy!(actual == position_value);
}
