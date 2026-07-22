use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR,
    RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use crate::math::fp::{Bps, Ray};
use crate::math::fp_core::mul_div_half_up;
use crate::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    protocol_fee_shares, simulate_update_indexes_body, update_borrow_index, update_supply_index,
    utilization,
};
use crate::types::{MarketParams, PoolStateRaw, PoolSyncData};

/// Seven-decimal token units per RAY-scaled share at index `RAY`.
const ASSET_TO_RAY_SCALE_7: i128 = 100_000_000_000_000_000_000;

fn valid_params(asset: Address) -> MarketParams {
    MarketParams {
        base_borrow_rate: Ray::from(RAY / 100),
        slope1: Ray::from(RAY / 10),
        slope2: Ray::from(RAY / 5),
        slope3: Ray::from(RAY / 2),
        mid_utilization: Ray::from(RAY / 2),
        optimal_utilization: Ray::from(RAY * 8 / 10),
        max_utilization: Ray::from(RAY * 95 / 100),
        max_borrow_rate: Ray::from(MAX_BORROW_RATE_RAY),
        reserve_factor: Bps::from(1_000),
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: 7,
    }
}

#[rule]
fn utilization_zero_when_supplied_zero(e: Env, borrowed: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));

    let util = utilization(&e, Ray::from(borrowed), Ray::ZERO);
    cvlr_assert!(util.raw() == 0);
}

#[rule]
fn utilization_bounded_when_borrowed_lte_supplied(e: Env, borrowed: i128, supplied: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((1..=100 * RAY).contains(&supplied));
    cvlr_assume!(borrowed <= supplied);

    let util = utilization(&e, Ray::from(borrowed), Ray::from(supplied));
    cvlr_assert!(util.raw() >= 0);
    cvlr_assert!(util.raw() <= RAY);
}

#[rule]
fn borrow_rate_per_ms_respects_annual_cap(e: Env, asset: Address, util_raw: i128) {
    cvlr_assume!((0..=RAY).contains(&util_raw));

    let params = valid_params(asset);
    let rate = calculate_borrow_rate(&e, Ray::from(util_raw), &params);
    let per_ms_cap = params
        .max_borrow_rate
        .div_by_int(MILLISECONDS_PER_YEAR as i128);
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= per_ms_cap.raw());
}

#[rule]
fn deposit_rate_zero_when_no_utilization(e: Env, borrow_rate: i128) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));

    let rate = calculate_deposit_rate(&e, Ray::ZERO, Ray::from(borrow_rate), Bps::from(1_000));
    cvlr_assert!(rate.raw() == 0);
}

#[rule]
fn deposit_rate_not_above_borrow_rate(e: Env, util_raw: i128, borrow_rate: i128, reserve_bps: u32) {
    cvlr_assume!((0..=RAY).contains(&util_raw));
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));
    cvlr_assume!(reserve_bps < BPS as u32);

    let rate = calculate_deposit_rate(
        &e,
        Ray::from(util_raw),
        Ray::from(borrow_rate),
        Bps::from(i128::from(reserve_bps)),
    );
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= borrow_rate);
}

#[rule]
fn compound_interest_identity_at_zero_delta(e: Env, rate: i128) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&rate));

    let factor = compound_interest(&e, Ray::from(rate), 0);
    cvlr_assert!(factor.raw() == RAY);
}

#[rule]
fn update_borrow_index_monotonic_when_factor_gte_one(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((RAY..=2 * RAY).contains(&factor));

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
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
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards),
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
    let old_debt = Ray::from(borrowed).mul(&e, Ray::from(old_index));
    let new_debt = Ray::from(borrowed).mul(&e, Ray::from(new_index));
    let accrued = new_debt.checked_sub(&e, old_debt);
    let (supplier, fee) = calculate_supplier_rewards(
        &e,
        &params,
        Ray::from(borrowed),
        Ray::from(new_index),
        Ray::from(old_index),
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

    let sync = PoolSyncData {
        params: (&valid_params(asset)).into(),
        state: PoolStateRaw {
            supplied,
            borrowed,
            revenue: 0,
            borrow_index,
            supply_index,
            last_timestamp: timestamp,
            cash: supplied
                .saturating_sub(borrowed)
                .checked_div(ASSET_TO_RAY_SCALE_7)
                .unwrap_or(0),
        },
    };
    let index = simulate_update_indexes_body(&e, timestamp, &sync);

    cvlr_assert!(index.borrow_index.raw() == borrow_index);
    cvlr_assert!(index.supply_index.raw() == supply_index);
}

/// Ceiling lemma: reachable-domain input (`>= floor`, `<= cap`) stays `<= cap`
/// for any rewards. Justifies the upper bound assumed by index summaries.
#[rule]
fn update_supply_index_capped(e: Env, supplied: i128, old_index: i128, rewards: i128) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old_index));
    cvlr_assume!(rewards >= 0);

    let out = update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards),
    );
    cvlr_assert!(out.raw() <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(out.raw() >= old_index.min(MAX_SUPPLY_INDEX_RAY));
}

/// Ceiling lemma: reachable-domain input (`>= RAY`, `<= cap`) stays `<= cap`.
/// Factor bounded so `old * factor` fits the `i128` intermediate.
#[rule]
fn update_borrow_index_capped(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old_index));
    cvlr_assume!((RAY..=10 * RAY).contains(&factor));

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
    cvlr_assert!(out.raw() <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(out.raw() >= RAY);
}

/// Virtual-offset defense: index growth is bounded by `old * rewards / V`
/// (`V = SUPPLY_VIRTUAL_VALUE_RAY = RAY`) regardless of how small `supplied`
/// is — a dust supplier donating rewards cannot inflate the index faster than
/// the phantom-base ratio allows.
#[rule]
fn update_supply_index_dust_growth_bounded(e: Env, supplied: i128, old_index: i128, rewards: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&supplied));
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((0..=10 * RAY).contains(&rewards));

    let out = update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards),
    );

    let max_growth = mul_div_half_up(&e, old_index, rewards, RAY);
    cvlr_assert!(out.raw() <= old_index + max_growth);
}

/// Fee-to-shares conversion never exceeds the `i128` headroom left in
/// `supplied` (a floored index post-wipeout cannot trap accrual), and stays
/// non-negative.
#[rule]
fn protocol_fee_shares_bounded_by_headroom(e: Env, fee: i128, supply_index: i128, supplied: i128) {
    cvlr_assume!(fee >= 0);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supplied >= 0);

    let out = protocol_fee_shares(
        &e,
        Ray::from(fee),
        Ray::from(supply_index),
        Ray::from(supplied),
    );
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= i128::MAX - supplied);
}

/// In-range conversion matches the plain half-up `fee / supply_index` divide.
#[rule]
fn protocol_fee_shares_matches_divide_in_range(
    e: Env,
    fee: i128,
    supply_index: i128,
    supplied: i128,
) {
    cvlr_assume!((0..=100 * RAY).contains(&fee));
    cvlr_assume!((RAY..=10 * RAY).contains(&supply_index));
    cvlr_assume!((0..=100 * RAY).contains(&supplied));

    let out = protocol_fee_shares(
        &e,
        Ray::from(fee),
        Ray::from(supply_index),
        Ray::from(supplied),
    );
    let plain = mul_div_half_up(&e, fee, RAY, supply_index);
    cvlr_assert!(out.raw() == plain);
}

// Summary bounds are compositionally justified by the lemmas above
// (end-to-end with symbolic `compound_interest` is SMT-intractable):
//   * borrow grows when factor >= RAY (e^x >= 1 for x >= 0) and stays <= cap
//   * supply grows for non-negative rewards and stays <= cap
//   * zero-delta early-return is exact

#[rule]
fn rates_reachability(e: Env, asset: Address) {
    let params = valid_params(asset);
    let rate = calculate_borrow_rate(&e, Ray::from(RAY / 2), &params);
    cvlr_satisfy!(rate.raw() > 0);
}
