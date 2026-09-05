//! Fixed-point behaviour at the edges of its domain.
//!
//! Two families that were proved through contract artifacts until 2026-09-03:
//! the boundary rules, which pin `calculate_borrow_rate` and `compound_interest`
//! at the exact corners of the rate curve and `fp_core` at the top of `i128`;
//! and the scaling round-trips, which bound the error of converting an amount
//! to shares and back. Neither family reads contract state or calls contract
//! code, so both belong to the crate that owns the arithmetic.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use crate::math::fp::{Bps, Ray};
use crate::math::fp_core::{div_by_int_half_up, mul_div_floor, mul_div_half_up, rescale_half_up};
use crate::rates::{calculate_borrow_rate, compound_interest};
use crate::types::MarketParams;

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
    let rate_per_ms = div_by_int_half_up(&e, RAY, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    cvlr_satisfy!(factor.raw() > 2 * RAY && factor.raw() < 3 * RAY);
}
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
    let rate_per_ms = div_by_int_half_up(&e, annual_rate_ray, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    let tolerance = RAY / 10_000;
    let lower = RAY + annual_rate_ray;

    cvlr_assert!(factor.raw() > RAY);
    cvlr_assert!(factor.raw() >= lower);
    cvlr_assert!(factor.raw() < lower + tolerance);
}
#[rule]
fn compound_taylor_accuracy_sanity(e: Env) {
    let rate_per_ms = div_by_int_half_up(&e, RAY / 100, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(rate_per_ms), MILLISECONDS_PER_YEAR);
    cvlr_satisfy!(factor.raw() > RAY + RAY / 100);
}
#[rule]
fn rescale_ray_to_wad(e: Env) {
    let result = rescale_half_up(&e, RAY, 27, 18);
    cvlr_assert!(result == WAD);
}
#[rule]
fn rescale_wad_to_7_decimals(e: Env) {
    let result = rescale_half_up(&e, WAD, 18, 7);
    cvlr_assert!(result == 10_000_000i128);
}
#[rule]
fn supply_dust_amount_sanity(e: Env) {
    let scaled = mul_div_half_up(&e, 1, RAY, RAY);
    cvlr_satisfy!(scaled == 1);
}
/// Amount → shares → amount through `mul_div_half_up` at any index in
/// `[1, 10]` RAY loses at most six units. Supply/withdraw and borrow/repay
/// are the same primitive with the same bounds, so one rule covers both.
#[rule]
fn supply_withdraw_roundtrip_error_bounded(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!((RAY..=10 * RAY).contains(&supply_index));

    let scaled = mul_div_half_up(&e, amount, RAY, supply_index);
    let recovered = mul_div_half_up(&e, scaled, supply_index, RAY);

    cvlr_assert!(recovered >= amount.saturating_sub(6));
    cvlr_assert!(recovered <= amount + 6);
}
/// Up to a 1000% annual rate over up to a year, the compounding factor never
/// wraps below one and never exceeds 100,000×.
#[rule]
fn compound_interest_bounded_output(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();

    let max_rate_per_ms = div_by_int_half_up(&e, 10 * RAY, MILLISECONDS_PER_YEAR as i128);

    cvlr_assume!(rate >= 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time <= MILLISECONDS_PER_YEAR);

    let factor = compound_interest(&e, Ray::from(rate), time);

    cvlr_assert!(factor.raw() >= RAY);
    cvlr_assert!(factor.raw() < 100_000 * RAY);
}
#[rule]
fn roundtrip_supply_sanity(e: Env) {
    let amount = WAD;
    let index = RAY;

    let scaled = mul_div_half_up(&e, amount, RAY, index);
    let recovered = mul_div_half_up(&e, scaled, index, RAY);
    cvlr_satisfy!(recovered == amount);
}
#[rule]
fn compound_no_wrap_sanity(e: Env) {
    let max_rate_per_ms = div_by_int_half_up(&e, RAY, MILLISECONDS_PER_YEAR as i128);
    let factor = compound_interest(&e, Ray::from(max_rate_per_ms), 1);
    cvlr_satisfy!(factor.raw() >= RAY);
}
#[rule]
fn scaled_to_actual_matches_floor_with_rounding(e: Env) {
    let scaled: i128 = cvlr::nondet::nondet();
    let index: i128 = cvlr::nondet::nondet();
    cvlr_assume!(scaled > 0 && scaled <= WAD * 1_000_000);
    cvlr_assume!((RAY..=10 * RAY).contains(&index));

    let actual = mul_div_half_up(&e, scaled, index, RAY);
    let floor = mul_div_floor(&e, scaled, index, RAY);

    cvlr_assert!(actual >= scaled);
    cvlr_assert!(actual >= floor);
    cvlr_assert!(actual <= floor + 1);
}
