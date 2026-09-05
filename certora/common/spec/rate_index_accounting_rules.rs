use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR,
    RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use crate::math::fp::{Bps, Ray};
use crate::rates::{
    calculate_borrow_rate, calculate_supplier_rewards, compound_interest,
    simulate_update_indexes_body, supply_index_reward_shortfall, update_borrow_index,
    update_supply_index, MAX_COMPOUND_DELTA_MS,
};
use crate::spec::harness::nondet_market_params;
use crate::types::{MarketIndex, MarketParams, PoolStateRaw, PoolSyncData};

const REWARD_REGRESSION_INDEX_MAX: i128 = 200_000_000 * RAY;

/// Scaled-share ceiling used by the index rules. One RAY of shares is one
/// whole token at index 1.0, so 100 RAY is a realistic book size that keeps
/// every `I256` intermediate far from the `i128` bound.
const MAX_SHARES: i128 = 100 * RAY;

/// Stored-index ceiling for the seeded market. One `MAX_COMPOUND_DELTA_MS`
/// chunk at the maximum borrow rate multiplies the borrow index by at most
/// `e^2 ≈ 7.4`, so projections stay well inside the configured index caps.
const MAX_SEED_INDEX: i128 = 2 * RAY;

/// Ledger-time ceiling: keeps `last_timestamp + elapsed` inside `u64` and the
/// projection inside a single compounding chunk.
const MAX_SEED_TIMESTAMP: u64 = u64::MAX / 4;

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

/// No lemma split: `calculate_annual_borrow_rate` runs three multiply-divides
/// whose branch conditions depend on which curve segment `utilization` lands in
/// (`utilization * slope1 / mid`, `excess * slope2 / range`,
/// `excess * slope3 / range`), so no single input bound settles them. Every
/// operand is already inside the validated curve domain
/// (`InterestRateModel::verify`), which is the under-approximation this rule
/// relies on instead.
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
                .div_by_int(&e, MILLISECONDS_PER_YEAR as i128)
                .raw()
    );
}

/// No lemma split, for the same reason as `borrow_rate_monotonic_across_utilization`:
/// four evaluations of the curve, each on a different segment.
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
    let at_full = calculate_borrow_rate(&e, Ray::ONE, &params);
    let expected_zero = Ray::from(base.min(max_rate)).div_by_int(&e, MILLISECONDS_PER_YEAR as i128);
    let expected_mid =
        Ray::from((base + slope1).min(max_rate)).div_by_int(&e, MILLISECONDS_PER_YEAR as i128);
    let expected_optimal = Ray::from((base + slope1 + slope2).min(max_rate))
        .div_by_int(&e, MILLISECONDS_PER_YEAR as i128);
    let expected_full = Ray::from((base + slope1 + slope2 + slope3).min(max_rate))
        .div_by_int(&e, MILLISECONDS_PER_YEAR as i128);

    cvlr_assert!(at_zero.raw() == expected_zero.raw());
    cvlr_assert!(at_mid.raw() == expected_mid.raw());
    cvlr_assert!(at_optimal.raw() == expected_optimal.raw());
    cvlr_assert!(at_full.raw() == expected_full.raw());
}

/// No lemma split: the Taylor series runs seven `pow.mul(x)` steps whose branch
/// conditions all move with `pow`, a value derived inside the loop. The
/// under-approximation here is the input domain instead: `rate_per_ms` is capped
/// at `MAX_BORROW_RATE_RAY` per millisecond and `delta_ms` at one year, which is
/// exactly `MAX_COMPOUND_DELTA_MS`, the largest chunk
/// `simulate_update_indexes_body` ever passes.
#[rule]
fn compound_factor_never_below_one(e: Env, rate_per_ms: i128, delta_ms: u64) {
    let max_per_ms = Ray::from(MAX_BORROW_RATE_RAY)
        .div_by_int(&e, MILLISECONDS_PER_YEAR as i128)
        .raw();
    cvlr_assume!(rate_per_ms >= 0 && rate_per_ms <= max_per_ms);
    cvlr_assume!(delta_ms <= MILLISECONDS_PER_YEAR);

    let factor = compound_interest(&e, Ray::from(rate_per_ms), delta_ms);
    cvlr_assert!(factor.raw() >= RAY);
    cvlr_assert!(delta_ms != 0 || factor.raw() == RAY);
    cvlr_assert!(rate_per_ms == 0 || delta_ms == 0 || factor.raw() > RAY);
}

/// No lemma split: `old_index >= RAY` makes `old_index * RAY` at least `1e54`,
/// so `mul_div_half_up` always widens to `I256` here. The native branch is
/// unreachable on this domain.
#[rule]
fn borrow_index_identity_is_noop(e: Env, old_index: i128) {
    cvlr_assume!(old_index >= RAY && old_index <= MAX_BORROW_INDEX_RAY);

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::ONE);
    cvlr_assert!(out.raw() == old_index);
}

/// No lemma split: both operands are at least `RAY`, so the product is at least
/// `1e54` and the widened path always wins.
#[rule]
fn borrow_index_strictly_grows_below_cap(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!(old_index >= RAY && old_index < MAX_BORROW_INDEX_RAY);
    cvlr_assume!(factor > RAY && factor <= 10 * RAY);

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
    cvlr_assert!(out.raw() > old_index);
    cvlr_assert!(out.raw() <= MAX_BORROW_INDEX_RAY);
}

/// No lemma split: the index is pinned at `MAX_BORROW_INDEX_RAY = 1e36` and the
/// factor is at least `RAY`, so the product is at least `1e63` -- always widened.
#[rule]
fn borrow_index_cap_is_sticky(e: Env, factor: i128) {
    cvlr_assume!(factor > RAY && factor <= 10 * RAY);

    let out = update_borrow_index(&e, Ray::from(MAX_BORROW_INDEX_RAY), Ray::from(factor));
    cvlr_assert!(out.raw() == MAX_BORROW_INDEX_RAY);
}

/// No lemma split: both calls return on the zero guard before any
/// multiply-divide runs.
#[rule]
fn supply_index_zero_inputs_are_noop(e: Env, supplied: i128, old_index: i128, rewards: i128) {
    cvlr_assume!(supplied >= 0 && supplied <= 100 * RAY);
    cvlr_assume!(old_index >= SUPPLY_INDEX_FLOOR_RAW && old_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(rewards >= 0 && rewards <= 100 * RAY);

    let no_supply = update_supply_index(&e, Ray::ZERO, Ray::from(old_index), Ray::from(rewards));
    let no_rewards = update_supply_index(&e, Ray::from(supplied), Ray::from(old_index), Ray::ZERO);

    cvlr_assert!(no_supply.raw() == old_index);
    cvlr_assert!(no_rewards.raw() == old_index);
}

/// No lemma split: the rule's own assume, `supplied * old_index` rounding to
/// zero, forces `supplied * old_index < RAY / 2 = 5e26`, far inside `i128`. The
/// widened branch is unreachable here, so the rule already sees one arithmetic
/// path.
#[rule]
fn supply_index_rounded_zero_value_is_noop(e: Env, supplied: i128, old_index: i128, rewards: i128) {
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(old_index >= SUPPLY_INDEX_FLOOR_RAW && old_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(rewards > 0 && rewards <= 100 * RAY);
    let supplied_ray = Ray::from(supplied);
    let old_index_ray = Ray::from(old_index);
    cvlr_assume!(supplied_ray.mul(&e, old_index_ray).raw() == 0);

    let out = update_supply_index(&e, supplied_ray, old_index_ray, Ray::from(rewards));
    cvlr_assert!(out.raw() == old_index);
}

/// No lemma split: `update_supply_index` runs `supplied * old_index` and then a
/// `mul_div_floor_saturating` on `new_value`, which is derived from the first
/// result. The rule then multiplies `supplied` by the *returned* index, whose
/// branch condition is not known before the call. No single input bound settles
/// all three, so a two-way split would leave both lemmas carrying a branch.
#[rule]
fn supply_index_reward_distribution_is_conservative(
    e: Env,
    supplied: i128,
    old_index: i128,
    rewards: i128,
) {
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(old_index >= SUPPLY_INDEX_FLOOR_RAW && old_index <= 10 * RAY);
    cvlr_assume!(rewards > 0 && rewards <= 100 * RAY);
    let supplied_ray = Ray::from(supplied);
    let old_index_ray = Ray::from(old_index);
    cvlr_assume!(supplied_ray.mul(&e, old_index_ray).raw() > 0);

    let reward_ray = Ray::from(rewards);
    let out = update_supply_index(&e, supplied_ray, old_index_ray, reward_ray);
    cvlr_assert!(out.raw() >= old_index && out.raw() <= MAX_SUPPLY_INDEX_RAY);
    let old_value = supplied_ray.mul(&e, old_index_ray);
    let new_value = supplied_ray.mul(&e, out);
    let distributed = new_value.checked_sub(&e, old_value);
    cvlr_assert!(distributed.raw() <= rewards);
    let shortfall = supply_index_reward_shortfall(&e, supplied_ray, old_index_ray, out, reward_ray);
    cvlr_assert!(distributed.raw() + shortfall.raw() == rewards);
}

/// No lemma split: `supplied` is pinned at `100 RAY = 1e29` and `old_index`
/// exceeds `10 RAY`, so every product here is at least `1e57` and the widened
/// path always wins.
#[rule]
fn supply_index_high_index_rewards_are_conservative(e: Env, old_index: i128, rewards: i128) {
    cvlr_assume!(old_index > 10 * RAY && old_index <= REWARD_REGRESSION_INDEX_MAX);
    cvlr_assume!(rewards > 0 && rewards <= 100 * RAY);
    let supplied = Ray::from(100 * RAY);
    let old_index_ray = Ray::from(old_index);
    let reward_ray = Ray::from(rewards);

    let out = update_supply_index(&e, supplied, old_index_ray, reward_ray);
    cvlr_assert!(out.raw() >= old_index && out.raw() <= MAX_SUPPLY_INDEX_RAY);
    let old_value = supplied.mul(&e, old_index_ray);
    let new_value = supplied.mul(&e, out);
    let distributed = new_value.checked_sub(&e, old_value);
    cvlr_assert!(distributed.raw() <= rewards);
    let shortfall = supply_index_reward_shortfall(&e, supplied, old_index_ray, out, reward_ray);

    cvlr_assert!(distributed.raw() + shortfall.raw() == rewards);
}

/// No lemma split: `supplied = RAY / 10` against an index at
/// `MAX_SUPPLY_INDEX_RAY` puts the product at `1e62` -- always widened.
#[rule]
fn supply_index_cap_is_sticky(e: Env, rewards: i128) {
    cvlr_assume!(rewards > 0 && rewards <= 100 * RAY);
    let supplied = Ray::from(RAY / 10);
    let old_index = Ray::from(MAX_SUPPLY_INDEX_RAY);
    let reward = Ray::from(rewards);

    let out = update_supply_index(&e, supplied, old_index, reward);
    cvlr_assert!(out.raw() == MAX_SUPPLY_INDEX_RAY);
    let shortfall = supply_index_reward_shortfall(&e, supplied, old_index, out, reward);

    cvlr_assert!(shortfall.raw() == rewards);
}

/// Native half of `accrued_interest_split_is_conservative`.
///
/// `calculate_supplier_rewards` runs two products, `borrowed * old_index` and
/// `borrowed * new_index`; the `apply_to_ray` that splits the accrued interest
/// multiplies a value below `1e30` by at most `BPS`, so it never leaves the
/// native path. `old_index <= new_index` orders the two products, so bounding
/// the larger one puts both on the native branch. `new_index.max(1)` keeps the
/// divisor total; the domain forces `new_index >= RAY`, so the clamp never binds.
///
/// This native half is a dust sliver of the domain (`new_index >= RAY` means it
/// needs `borrowed <= ~170` ray-shares), which is exactly the point: the widened
/// lemma then covers every economically reachable input with the compiler-rt
/// limb code removed from its path.
#[rule]
#[allow(clippy::too_many_arguments)]
fn accrued_interest_split_is_conservative_native(
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
    cvlr_assume!(borrowed <= (i128::MAX - RAY / 2) / new_index.max(1));

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

/// Widened half of `accrued_interest_split_is_conservative`: `borrowed *
/// new_index` overflows `i128`, so the debt valuations run as exact `I256` host
/// calls. Exact complement of the native lemma's bound, so the pair covers the
/// original domain.
#[rule]
#[allow(clippy::too_many_arguments)]
fn accrued_interest_split_is_conservative_widened(
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
    cvlr_assume!(borrowed > (i128::MAX - RAY / 2) / new_index.max(1));

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

// ---------------------------------------------------------------------------
// Index-projection rules, moved here from the controller layer on 2026-09-03.
//
// These rules feed a symbolic market and a symbolic accrual window into the
// projection. They call `simulate_update_indexes_body`, not the public
// `simulate_update_indexes`: under this crate's `certora` feature the public
// wrapper is replaced by `simulate_update_indexes_summary`, whose outputs are
// independent nondets bounded only from below, so the equality and ordering
// asserted below would be spurious counterexamples. On the controller artifact
// the public wrapper was the real body because `controller/certora` does not
// enable `common/certora`.
// ---------------------------------------------------------------------------

/// A symbolic market as the pool would report it through `get_sync_data`,
/// last accrued at `last_timestamp`.
fn nondet_sync(asset: &Address, last_timestamp: u64) -> PoolSyncData {
    let supplied: i128 = cvlr::nondet::nondet();
    let borrowed: i128 = cvlr::nondet::nondet();
    let revenue: i128 = cvlr::nondet::nondet();
    let cash: i128 = cvlr::nondet::nondet();
    let borrow_index: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=MAX_SHARES).contains(&supplied));
    cvlr_assume!((0..=supplied).contains(&borrowed));
    cvlr_assume!((0..=supplied).contains(&revenue));
    cvlr_assume!((0..=MAX_SHARES).contains(&cash));
    cvlr_assume!((RAY..=MAX_SEED_INDEX).contains(&borrow_index));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SEED_INDEX).contains(&supply_index));

    PoolSyncData {
        params: nondet_market_params(asset),
        state: PoolStateRaw {
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
            last_timestamp,
            cash,
        },
    }
}
/// The market state `update_indexes` commits at `now`: indexes replaced by the
/// projection, `last_timestamp` stamped, and protocol fee shares minted into
/// both `supplied` and `revenue`.
///
/// The minted share count is left symbolic rather than recomputed, so the rule
/// holds for *any* fee the pool could have booked — strictly stronger than
/// pinning production's exact `protocol_fee_shares` result.
fn accrued_sync(sync: &PoolSyncData, projected: &MarketIndex, now: u64) -> PoolSyncData {
    let minted: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=MAX_SHARES).contains(&minted));

    PoolSyncData {
        params: sync.params.clone(),
        state: PoolStateRaw {
            supplied: sync.state.supplied + minted,
            borrowed: sync.state.borrowed,
            revenue: sync.state.revenue + minted,
            borrow_index: projected.borrow_index.raw(),
            supply_index: projected.supply_index.raw(),
            last_timestamp: now,
            cash: sync.state.cash,
        },
    }
}
/// Draws a `(last_timestamp, now)` pair with `now - last_timestamp` inside one
/// compounding chunk, so `simulate_update_indexes` runs a single iteration.
fn nondet_accrual_window() -> (u64, u64) {
    let last_timestamp: u64 = cvlr::nondet::nondet();
    let elapsed_ms: u64 = cvlr::nondet::nondet();
    cvlr_assume!(last_timestamp <= MAX_SEED_TIMESTAMP);
    cvlr_assume!(elapsed_ms <= MAX_COMPOUND_DELTA_MS);
    (last_timestamp, last_timestamp + elapsed_ms)
}
#[rule]
fn indexes_unchanged_when_no_time_elapsed(e: Env) {
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let old_supply_index: i128 = cvlr::nondet::nondet();
    let supplied: i128 = cvlr::nondet::nondet();
    let rate: i128 = cvlr::nondet::nondet();

    cvlr_assume!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old_borrow_index));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old_supply_index));
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(rate >= 0);

    let factor = crate::rates::compound_interest(&e, Ray::from(rate), 0);
    cvlr_assert!(factor == Ray::ONE);

    let new_borrow = crate::rates::update_borrow_index(&e, Ray::from(old_borrow_index), factor);
    cvlr_assert!(new_borrow.raw() == old_borrow_index);

    let new_supply = crate::rates::update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_supply_index),
        Ray::ZERO,
    );
    cvlr_assert!(new_supply.raw() == old_supply_index);
}
/// `get_market_index` returns the same pair whether or not `update_indexes`
/// ran first.
///
/// The Blackthorn L-6 / Certora Hub L-03 shape: a view that reads unaccrued
/// state disagrees with the mutating path that accrues first, so a position
/// looks healthier (or riskier) than it is. Here both sides are the *same*
/// projection: reading before accrual projects the stored state forward to
/// `now`; reading after accrual re-projects a state already stamped at `now`,
/// which the zero-delta early return leaves untouched.
#[rule]
fn iso_market_index_invariant_across_accrual(e: Env, asset: Address) {
    let (last_timestamp, now) = nondet_accrual_window();
    let sync = nondet_sync(&asset, last_timestamp);

    // What a view returns with no prior `update_indexes`.
    let before = simulate_update_indexes_body(&e, now, &sync);

    // What the same view returns immediately after `update_indexes`.
    let accrued = accrued_sync(&sync, &before, now);
    let after = simulate_update_indexes_body(&e, now, &accrued);

    cvlr_assert!(after.borrow_index.raw() == before.borrow_index.raw());
    cvlr_assert!(after.supply_index.raw() == before.supply_index.raw());
    cvlr_assert!(after.borrow_index.raw() <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(after.supply_index.raw() <= MAX_SUPPLY_INDEX_RAY);
}
/// `get_market_index` is monotone in ledger time when nothing accrues between
/// the two reads: a keeper who waits never sees a smaller index.
///
/// Both projections start from the *same* stored state, which is exactly the
/// "no accrual is invoked" precondition — `update_indexes` is what would move
/// `last_timestamp` forward.
#[rule]
fn time_mono_market_index_non_decreasing(e: Env, asset: Address) {
    let last_timestamp: u64 = cvlr::nondet::nondet();
    let early_delta: u64 = cvlr::nondet::nondet();
    let late_delta: u64 = cvlr::nondet::nondet();
    cvlr_assume!(last_timestamp <= MAX_SEED_TIMESTAMP);
    cvlr_assume!(early_delta <= late_delta);
    cvlr_assume!(late_delta <= MAX_COMPOUND_DELTA_MS);

    let sync = nondet_sync(&asset, last_timestamp);

    let early = simulate_update_indexes_body(&e, last_timestamp + early_delta, &sync);
    let late = simulate_update_indexes_body(&e, last_timestamp + late_delta, &sync);

    cvlr_assert!(late.borrow_index.raw() >= early.borrow_index.raw());
    cvlr_assert!(late.supply_index.raw() >= early.supply_index.raw());
}
