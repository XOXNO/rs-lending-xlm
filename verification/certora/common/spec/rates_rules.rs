use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY};
use crate::fp::Ray;
use crate::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    simulate_update_indexes, update_borrow_index, update_supply_index, utilization,
};
use crate::types::MarketParams;

fn valid_params(asset: Address) -> MarketParams {
    MarketParams {
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_borrow_rate_ray: MAX_BORROW_RATE_RAY,
        reserve_factor_bps: 1_000,
        asset_id: asset,
        asset_decimals: 7,
    }
}

#[rule]
fn utilization_zero_when_supplied_zero(e: Env, borrowed: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));

    let util = utilization(&e, Ray::from_raw(borrowed), Ray::ZERO);
    cvlr_assert!(util.raw() == 0);
}

#[rule]
fn utilization_bounded_when_borrowed_lte_supplied(e: Env, borrowed: i128, supplied: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((1..=100 * RAY).contains(&supplied));
    cvlr_assume!(borrowed <= supplied);

    let util = utilization(&e, Ray::from_raw(borrowed), Ray::from_raw(supplied));
    cvlr_assert!(util.raw() >= 0);
    cvlr_assert!(util.raw() <= RAY);
}

#[rule]
fn borrow_rate_is_capped(e: Env, asset: Address, util_raw: i128) {
    cvlr_assume!((0..=RAY).contains(&util_raw));

    let params = valid_params(asset);
    let rate = calculate_borrow_rate(&e, Ray::from_raw(util_raw), &params);
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= params.max_borrow_rate_ray);
}

#[rule]
fn deposit_rate_zero_when_no_utilization(e: Env, borrow_rate: i128) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));

    let rate = calculate_deposit_rate(&e, Ray::ZERO, Ray::from_raw(borrow_rate), 1_000);
    cvlr_assert!(rate.raw() == 0);
}

#[rule]
fn deposit_rate_not_above_borrow_rate(e: Env, util_raw: i128, borrow_rate: i128, reserve_bps: u32) {
    cvlr_assume!((0..=RAY).contains(&util_raw));
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));
    cvlr_assume!(reserve_bps < BPS as u32);

    let rate = calculate_deposit_rate(
        &e,
        Ray::from_raw(util_raw),
        Ray::from_raw(borrow_rate),
        reserve_bps,
    );
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= borrow_rate);
}

#[rule]
fn compound_interest_identity_at_zero_delta(e: Env, rate: i128) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&rate));

    let factor = compound_interest(&e, Ray::from_raw(rate), 0);
    cvlr_assert!(factor.raw() == RAY);
}

#[rule]
fn update_borrow_index_monotonic_when_factor_gte_one(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((RAY..=2 * RAY).contains(&factor));

    let out = update_borrow_index(&e, Ray::from_raw(old_index), Ray::from_raw(factor));
    cvlr_assert!(out.raw() >= old_index);
}

#[rule]
fn update_supply_index_monotonic_when_rewards_positive(
    e: Env,
    supplied: i128,
    old_index: i128,
    rewards: i128,
) {
    cvlr_assume!((1..=100 * RAY).contains(&supplied));
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((0..=10 * RAY).contains(&rewards));

    let out = update_supply_index(
        &e,
        Ray::from_raw(supplied),
        Ray::from_raw(old_index),
        Ray::from_raw(rewards),
    );
    cvlr_assert!(out.raw() >= old_index);
}

#[rule]
fn supplier_rewards_plus_fee_equals_accrued_interest(
    e: Env,
    asset: Address,
    borrowed: i128,
    old_index: i128,
    new_index: i128,
) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((old_index..=10 * RAY).contains(&new_index));

    let params = valid_params(asset);
    let old_debt = Ray::from_raw(borrowed).mul(&e, Ray::from_raw(old_index));
    let new_debt = Ray::from_raw(borrowed).mul(&e, Ray::from_raw(new_index));
    let accrued = new_debt - old_debt;
    let (supplier, fee) = calculate_supplier_rewards(
        &e,
        &params,
        Ray::from_raw(borrowed),
        Ray::from_raw(new_index),
        Ray::from_raw(old_index),
    );

    cvlr_assert!(supplier.raw() >= 0);
    cvlr_assert!(fee.raw() >= 0);
    cvlr_assert!(supplier.raw() + fee.raw() == accrued.raw());
}

#[rule]
fn simulate_indexes_no_time_noop(
    e: Env,
    asset: Address,
    borrowed: i128,
    supplied: i128,
    borrow_index: i128,
    supply_index: i128,
    timestamp: u64,
) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((0..=100 * RAY).contains(&supplied));
    cvlr_assume!((RAY..=10 * RAY).contains(&borrow_index));
    cvlr_assume!((RAY..=10 * RAY).contains(&supply_index));

    let index = simulate_update_indexes(
        &e,
        timestamp,
        timestamp,
        Ray::from_raw(borrowed),
        Ray::from_raw(borrow_index),
        Ray::from_raw(supplied),
        Ray::from_raw(supply_index),
        &valid_params(asset),
    );

    cvlr_assert!(index.borrow_index_ray == borrow_index);
    cvlr_assert!(index.supply_index_ray == supply_index);
}

#[rule]
fn rates_reachability(e: Env, asset: Address) {
    let params = valid_params(asset);
    let rate = calculate_borrow_rate(&e, Ray::from_raw(RAY / 2), &params);
    cvlr_satisfy!(rate.raw() > 0);
}
