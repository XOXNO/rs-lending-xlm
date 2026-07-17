//! Interest rate model rules: borrow/deposit rates, compound interest, rewards, and index updates.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use crate::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY, WAD};
use crate::types::MarketParams;
use common::math::fp::Ray;
use common::math::fp_core::{div_by_int_half_up, mul_div_half_up};
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    update_borrow_index, update_supply_index,
};

fn nondet_valid_params(e: &Env) -> MarketParams {
    let base_borrow_rate: i128 = cvlr::nondet::nondet();
    let slope1: i128 = cvlr::nondet::nondet();
    let slope2: i128 = cvlr::nondet::nondet();
    let slope3: i128 = cvlr::nondet::nondet();
    let mid_utilization: i128 = cvlr::nondet::nondet();
    let optimal_utilization: i128 = cvlr::nondet::nondet();
    let max_utilization: i128 = cvlr::nondet::nondet();
    cvlr_assume!(max_utilization >= optimal_utilization && max_utilization <= RAY);
    let max_borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor: u32 = cvlr::nondet::nondet();
    let asset_id = e.current_contract_address();
    let asset_decimals: u32 = cvlr::nondet::nondet();

    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&base_borrow_rate));
    cvlr_assume!(slope1 <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(slope2 <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(slope3 <= MAX_BORROW_RATE_RAY);

    cvlr_assume!(base_borrow_rate <= slope1);
    cvlr_assume!(slope1 <= slope2);
    cvlr_assume!(slope2 <= slope3);

    cvlr_assume!(mid_utilization > 0 && mid_utilization < optimal_utilization);
    cvlr_assume!(optimal_utilization < RAY);

    cvlr_assume!(max_borrow_rate > 0 && max_borrow_rate <= MAX_BORROW_RATE_RAY);

    cvlr_assume!((0..BPS).contains(&i128::from(reserve_factor)));

    cvlr_assume!(asset_decimals <= 27);

    MarketParams {
        base_borrow_rate: Ray::from(base_borrow_rate),
        slope1: Ray::from(slope1),
        slope2: Ray::from(slope2),
        slope3: Ray::from(slope3),
        mid_utilization: Ray::from(mid_utilization),
        optimal_utilization: Ray::from(optimal_utilization),
        max_utilization: Ray::from(max_utilization),
        max_borrow_rate: Ray::from(max_borrow_rate),
        reserve_factor: common::math::fp::Bps::from(i128::from(reserve_factor)),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id,
        asset_decimals,
    }
}

#[rule]
fn borrow_rate_zero_utilization(e: Env) {
    let params = nondet_valid_params(&e);

    let rate = calculate_borrow_rate(&e, Ray::ZERO, &params);

    let annual = if params.base_borrow_rate > params.max_borrow_rate {
        params.max_borrow_rate.raw()
    } else {
        params.base_borrow_rate.raw()
    };
    let expected = div_by_int_half_up(annual, MILLISECONDS_PER_YEAR as i128);

    cvlr_assert!(rate.raw() == expected);
}

#[rule]
fn borrow_rate_monotonic(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&util_a));
    cvlr_assume!((0..=RAY).contains(&util_b));
    cvlr_assume!(util_a < util_b);

    let rate_a = calculate_borrow_rate(&e, Ray::from(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

#[rule]
fn borrow_rate_monotonic_in_region1(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(util_a >= 0);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b < params.mid_utilization.raw());

    let rate_a = calculate_borrow_rate(&e, Ray::from(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

#[rule]
fn borrow_rate_monotonic_in_region2(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(params.mid_utilization.raw() <= util_a);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b < params.optimal_utilization.raw());

    let rate_a = calculate_borrow_rate(&e, Ray::from(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

#[rule]
fn borrow_rate_monotonic_in_region3(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(params.optimal_utilization.raw() <= util_a);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b <= RAY);

    let rate_a = calculate_borrow_rate(&e, Ray::from(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

/// Borrow rate never exceeds max_borrow_rate per millisecond and stays non-negative.
#[rule]
fn borrow_rate_capped(e: Env) {
    let params = nondet_valid_params(&e);

    let utilization: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&utilization));

    let rate = calculate_borrow_rate(&e, Ray::from(utilization), &params);
    let cap = div_by_int_half_up(params.max_borrow_rate.raw(), MILLISECONDS_PER_YEAR as i128);

    cvlr_assert!(rate.raw() <= cap + 1);
    cvlr_assert!(rate.raw() >= 0);
}

#[rule]
fn borrow_rate_continuity_at_mid(e: Env) {
    let params = nondet_valid_params(&e);

    cvlr_assume!(params.mid_utilization.raw() >= 2);

    let rate_below =
        calculate_borrow_rate(&e, Ray::from(params.mid_utilization.raw() - 1), &params);
    let rate_at = calculate_borrow_rate(&e, params.mid_utilization, &params);

    let diff = if rate_at >= rate_below {
        rate_at.raw() - rate_below.raw()
    } else {
        rate_below.raw() - rate_at.raw()
    };

    cvlr_assert!(diff <= 1);
}

#[rule]
fn borrow_rate_continuity_at_optimal(e: Env) {
    let params = nondet_valid_params(&e);

    cvlr_assume!(params.optimal_utilization.raw() >= 2);

    let rate_below =
        calculate_borrow_rate(&e, Ray::from(params.optimal_utilization.raw() - 1), &params);
    let rate_at = calculate_borrow_rate(&e, params.optimal_utilization, &params);

    let diff = if rate_at >= rate_below {
        rate_at.raw() - rate_below.raw()
    } else {
        rate_below.raw() - rate_at.raw()
    };

    cvlr_assert!(diff <= 1);
}

#[rule]
fn deposit_rate_zero_when_no_utilization(e: Env) {
    let borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor: u32 = cvlr::nondet::nondet();

    cvlr_assume!(borrow_rate >= 0);
    cvlr_assume!((0..BPS).contains(&i128::from(reserve_factor)));

    let rate = calculate_deposit_rate(
        &e,
        Ray::ZERO,
        Ray::from(borrow_rate),
        common::math::fp::Bps::from(i128::from(reserve_factor)),
    );

    cvlr_assert!(rate == Ray::ZERO);
}

#[rule]
fn deposit_rate_less_than_borrow(e: Env) {
    let utilization: i128 = cvlr::nondet::nondet();
    let borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor: u32 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&utilization));
    cvlr_assume!((0..=RAY).contains(&borrow_rate));
    cvlr_assume!((0..BPS).contains(&i128::from(reserve_factor)));

    let deposit_rate = calculate_deposit_rate(
        &e,
        Ray::from(utilization),
        Ray::from(borrow_rate),
        common::math::fp::Bps::from(i128::from(reserve_factor)),
    );
    let upper_bound = mul_div_half_up(&e, utilization, borrow_rate, RAY);

    cvlr_assert!(deposit_rate.raw() <= upper_bound + 1);
}

#[rule]
fn compound_interest_identity(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&rate));

    let factor = compound_interest(&e, Ray::from(rate), 0);

    cvlr_assert!(factor == Ray::ONE);
}

#[rule]
fn compound_interest_monotonic_in_time(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let t1: u64 = cvlr::nondet::nondet();
    let t2: u64 = cvlr::nondet::nondet();

    cvlr_assume!(rate >= 0);
    cvlr_assume!(rate <= div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128));
    cvlr_assume!(t1 > 0);
    cvlr_assume!(t1 < t2);
    cvlr_assume!(t2 <= MILLISECONDS_PER_YEAR);

    let factor1 = compound_interest(&e, Ray::from(rate), t1);
    let factor2 = compound_interest(&e, Ray::from(rate), t2);

    cvlr_assert!(factor2 >= factor1);
}

#[rule]
fn compound_interest_monotonic_in_rate(e: Env) {
    let r1: i128 = cvlr::nondet::nondet();
    let r2: i128 = cvlr::nondet::nondet();
    let t: u64 = cvlr::nondet::nondet();

    cvlr_assume!(r1 > 0);
    cvlr_assume!(r1 < r2);
    cvlr_assume!(r2 <= div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128));
    cvlr_assume!(t > 0 && t <= MILLISECONDS_PER_YEAR);

    let factor1 = compound_interest(&e, Ray::from(r1), t);
    let factor2 = compound_interest(&e, Ray::from(r2), t);

    cvlr_assert!(factor2 >= factor1);
}

#[rule]
fn compound_interest_ge_simple(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let t: u64 = cvlr::nondet::nondet();

    let max_rate = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    cvlr_assume!(rate >= 0 && rate <= max_rate);
    cvlr_assume!(t > 0 && t <= MILLISECONDS_PER_YEAR);

    let factor = compound_interest(&e, Ray::from(rate), t);

    let x = rate * (t as i128);
    let simple = RAY + x;

    cvlr_assert!(factor.raw() >= simple - 2);
}

#[rule]
fn supplier_rewards_conservation(e: Env) {
    let params = nondet_valid_params(&e);

    let borrowed: i128 = cvlr::nondet::nondet();
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let new_borrow_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(borrowed > 0);
    cvlr_assume!(old_borrow_index >= RAY);
    cvlr_assume!(new_borrow_index >= old_borrow_index);
    cvlr_assume!(borrowed < WAD);
    cvlr_assume!(new_borrow_index <= RAY * 8);

    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        &e,
        &params,
        Ray::from(borrowed),
        Ray::from(new_borrow_index),
        Ray::from(old_borrow_index),
    );

    let old_debt = mul_div_half_up(&e, borrowed, old_borrow_index, RAY);
    let new_debt = mul_div_half_up(&e, borrowed, new_borrow_index, RAY);
    let accrued_interest = new_debt - old_debt;

    let sum = supplier_rewards.raw() + protocol_fee.raw();
    let diff = if sum >= accrued_interest {
        sum - accrued_interest
    } else {
        accrued_interest - sum
    };

    cvlr_assert!(diff <= 1);

    let expected_fee = mul_div_half_up(&e, accrued_interest, params.reserve_factor.raw(), BPS);
    let fee_diff = if protocol_fee.raw() >= expected_fee {
        protocol_fee.raw() - expected_fee
    } else {
        expected_fee - protocol_fee.raw()
    };
    cvlr_assert!(fee_diff <= 1);
}

#[rule]
fn update_borrow_index_monotonic(e: Env) {
    let old_index: i128 = cvlr::nondet::nondet();
    let interest_factor: i128 = cvlr::nondet::nondet();

    cvlr_assume!(old_index >= RAY);
    cvlr_assume!(interest_factor >= RAY);
    cvlr_assume!(old_index <= RAY * 8);
    cvlr_assume!(interest_factor <= RAY * 8);

    let new_index = update_borrow_index(&e, Ray::from(old_index), Ray::from(interest_factor));

    cvlr_assert!(new_index.raw() >= old_index);
}

#[rule]
fn update_supply_index_monotonic(e: Env) {
    let supplied: i128 = cvlr::nondet::nondet();
    let old_index: i128 = cvlr::nondet::nondet();
    let rewards_increase: i128 = cvlr::nondet::nondet();

    cvlr_assume!(supplied >= 0);
    cvlr_assume!(old_index >= RAY);
    cvlr_assume!(rewards_increase >= 0);
    cvlr_assume!(supplied < WAD);
    cvlr_assume!(old_index <= RAY * 8);
    cvlr_assume!(rewards_increase < WAD);

    let new_index = update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards_increase),
    );

    cvlr_assert!(new_index.raw() >= old_index);
}
