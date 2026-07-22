//! Pure production rate/index lemmas used by pool accrual.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR,
    RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::{Bps, Ray};
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    update_borrow_index, update_supply_index, utilization,
};
use common::types::MarketParams;

#[allow(clippy::too_many_arguments)]
fn assume_valid_curve(
    base: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    mid: i128,
    optimal: i128,
    max_rate: i128,
) {
    cvlr_assume!(base >= 0);
    cvlr_assume!(base <= slope1 && slope1 <= slope2 && slope2 <= slope3);
    cvlr_assume!(slope3 <= max_rate);
    cvlr_assume!(max_rate > base && max_rate <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(mid > 0 && mid < optimal && optimal < RAY);
}

#[allow(clippy::too_many_arguments)]
fn curve(
    asset: Address,
    base: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    mid: i128,
    optimal: i128,
    max_rate: i128,
    reserve_factor: u32,
) -> MarketParams {
    MarketParams {
        max_borrow_rate: Ray::from(max_rate),
        base_borrow_rate: Ray::from(base),
        slope1: Ray::from(slope1),
        slope2: Ray::from(slope2),
        slope3: Ray::from(slope3),
        mid_utilization: Ray::from(mid),
        optimal_utilization: Ray::from(optimal),
        max_utilization: Ray::ONE,
        reserve_factor: Bps::from(i128::from(reserve_factor)),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: 7,
    }
}

/// The validated three-segment curve never falls as utilization crosses either kink.
#[rule]
#[allow(clippy::too_many_arguments)]
fn borrow_rate_monotonic_across_utilization(
    e: Env,
    asset: Address,
    lower_util: i128,
    upper_util: i128,
    base: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    mid: i128,
    optimal: i128,
    max_rate: i128,
) {
    assume_valid_curve(base, slope1, slope2, slope3, mid, optimal, max_rate);
    cvlr_assume!(lower_util >= 0 && lower_util <= upper_util && upper_util <= RAY);

    let params = curve(
        asset, base, slope1, slope2, slope3, mid, optimal, max_rate, 0,
    );
    let lower = calculate_borrow_rate(&e, Ray::from(lower_util), &params);
    let upper = calculate_borrow_rate(&e, Ray::from(upper_util), &params);

    cvlr_assert!(lower.raw() >= 0);
    cvlr_assert!(lower.raw() <= upper.raw());
    cvlr_assert!(
        upper.raw()
            <= params
                .max_borrow_rate
                .div_by_int(MILLISECONDS_PER_YEAR as i128)
                .raw()
    );
}

/// Both kinks join the adjacent segments at the configured cumulative rates.
#[rule]
#[allow(clippy::too_many_arguments)]
fn borrow_rate_kinks_match_configured_curve(
    e: Env,
    asset: Address,
    base: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    mid: i128,
    optimal: i128,
    max_rate: i128,
) {
    assume_valid_curve(base, slope1, slope2, slope3, mid, optimal, max_rate);
    let params = curve(
        asset, base, slope1, slope2, slope3, mid, optimal, max_rate, 0,
    );

    let at_zero = calculate_borrow_rate(&e, Ray::ZERO, &params);
    let at_mid = calculate_borrow_rate(&e, Ray::from(mid), &params);
    let at_optimal = calculate_borrow_rate(&e, Ray::from(optimal), &params);
    let expected_zero = Ray::from(base.min(max_rate)).div_by_int(MILLISECONDS_PER_YEAR as i128);
    let expected_mid =
        Ray::from((base + slope1).min(max_rate)).div_by_int(MILLISECONDS_PER_YEAR as i128);
    let expected_optimal =
        Ray::from((base + slope1 + slope2).min(max_rate)).div_by_int(MILLISECONDS_PER_YEAR as i128);

    cvlr_assert!(at_zero.raw() == expected_zero.raw());
    cvlr_assert!(at_mid.raw() == expected_mid.raw());
    cvlr_assert!(at_optimal.raw() == expected_optimal.raw());
}

/// Supplier rate remains nonnegative and cannot exceed the borrow rate.
#[rule]
fn deposit_rate_respects_utilization_and_reserve(
    e: Env,
    util_raw: i128,
    borrow_rate: i128,
    reserve_factor: u32,
) {
    cvlr_assume!(util_raw >= 0 && util_raw <= RAY);
    cvlr_assume!(borrow_rate >= 0 && borrow_rate <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(reserve_factor < BPS as u32);

    let out = calculate_deposit_rate(
        &e,
        Ray::from(util_raw),
        Ray::from(borrow_rate),
        Bps::from(i128::from(reserve_factor)),
    );
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= borrow_rate);
    cvlr_assert!(util_raw != 0 || out.raw() == 0);
}

/// Valid nonnegative per-millisecond rates produce a factor at least one.
#[rule]
fn compound_factor_never_below_one(e: Env, rate_per_ms: i128, delta_ms: u64) {
    let max_per_ms = Ray::from(MAX_BORROW_RATE_RAY)
        .div_by_int(MILLISECONDS_PER_YEAR as i128)
        .raw();
    cvlr_assume!(rate_per_ms >= 0 && rate_per_ms <= max_per_ms);
    cvlr_assume!(delta_ms <= MILLISECONDS_PER_YEAR);

    let factor = compound_interest(&e, Ray::from(rate_per_ms), delta_ms);
    cvlr_assert!(factor.raw() >= RAY);
    cvlr_assert!(delta_ms != 0 || factor.raw() == RAY);
}

/// Borrow index is monotone and saturates at its production cap.
#[rule]
fn borrow_index_grows_and_respects_cap(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!(old_index >= RAY && old_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(factor >= RAY && factor <= 10 * RAY);

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
    cvlr_assert!(out.raw() >= old_index);
    cvlr_assert!(out.raw() <= MAX_BORROW_INDEX_RAY);
}

/// Positive rewards never reduce the supply index and cannot exceed its cap.
#[rule]
fn supply_index_rewards_only_grow_and_respect_cap(
    e: Env,
    supplied: i128,
    old_index: i128,
    rewards: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 100 * RAY);
    cvlr_assume!(old_index >= SUPPLY_INDEX_FLOOR_RAW && old_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(rewards >= 0 && rewards <= 100 * RAY);

    let out = update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards),
    );
    cvlr_assert!(out.raw() >= old_index);
    cvlr_assert!(out.raw() <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!((supplied != 0 && rewards != 0) || out.raw() == old_index);
}

/// Debt-index growth is split exactly between suppliers and protocol revenue.
#[rule]
fn accrued_interest_split_is_conservative(
    e: Env,
    asset: Address,
    borrowed: i128,
    old_index: i128,
    new_index: i128,
    reserve_factor: u32,
) {
    cvlr_assume!(borrowed >= 0 && borrowed <= 100 * RAY);
    cvlr_assume!(old_index >= RAY && old_index <= new_index);
    cvlr_assume!(new_index <= 10 * RAY);
    cvlr_assume!(reserve_factor < BPS as u32);

    let params = curve(
        asset,
        RAY / 100,
        RAY / 10,
        RAY / 5,
        RAY / 2,
        RAY / 2,
        RAY * 8 / 10,
        MAX_BORROW_RATE_RAY,
        reserve_factor,
    );
    let (supplier, fee) = calculate_supplier_rewards(
        &e,
        &params,
        Ray::from(borrowed),
        Ray::from(new_index),
        Ray::from(old_index),
    );
    let old_debt = Ray::from(borrowed).mul(&e, Ray::from(old_index));
    let new_debt = Ray::from(borrowed).mul(&e, Ray::from(new_index));
    let accrued = new_debt.checked_sub(&e, old_debt);

    cvlr_assert!(supplier.raw() >= 0 && fee.raw() >= 0);
    cvlr_assert!(supplier.raw() + fee.raw() == accrued.raw());
}

/// Utilization is zero without supply and bounded for debt not above supply.
#[rule]
fn utilization_respects_supply_ratio(e: Env, borrowed: i128, supplied: i128) {
    cvlr_assume!(borrowed >= 0 && borrowed <= 100 * RAY);
    cvlr_assume!(supplied >= 0 && supplied <= 100 * RAY);
    cvlr_assume!(supplied == 0 || borrowed <= supplied);

    let out = utilization(&e, Ray::from(borrowed), Ray::from(supplied));
    cvlr_assert!(out.raw() >= 0 && out.raw() <= RAY);
    cvlr_assert!(supplied != 0 || out.raw() == 0);
}
