use cvlr::macros::rule;
use cvlr::prelude::clog;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{
    BPS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_SUPPLY_INDEX_RAY, RAY,
    SUPPLY_INDEX_FLOOR_RAW,
};
use crate::math::fp::{Bps, Ray};
use crate::math::fp_core::{mul_div_floor, mul_div_half_up};
use crate::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    protocol_fee_shares, simulate_update_indexes_body, update_borrow_index, update_supply_index,
    utilization,
};
use crate::types::{MarketParams, PoolStateRaw, PoolSyncData};

const ASSET_TO_RAY_SCALE_7: i128 = 100_000_000_000_000_000_000;

/// Production ceiling on any ray-denominated amount: a market total, a position
/// value, or an accrued interest amount.
///
/// `docs/reference/numeric-bounds.md` §3 derives the protocol-wide balance
/// ceiling from `Ray::from_asset`: no balance at any decimal count can exceed
/// `i128::MAX / RAY = 170_141_183_460.47` whole tokens, and the same bound
/// applies to a market total and to the `scaled * index` value of a position.
/// 1e11 whole tokens in ray is the largest round number strictly inside that
/// domain (`1e11 * RAY = 1e38 < i128::MAX = 1.7014e38`).
///
/// What the bound excludes: ray values above 1e11 whole tokens, which
/// `Ray::from_asset` cannot construct from any token amount and which no
/// accrual can reach without the surrounding `checked_*` first panicking. The
/// tightest configured supply cap on mainnet is AQUA at 5e8 whole tokens
/// (numeric-bounds §3), roughly 200x below this bound.
const MAX_RAY_VALUE: i128 = 100_000_000_000 * RAY;

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

/// No lemma split: `supplied == 0` makes `utilization` return before any
/// multiply-divide, so this rule has no arithmetic branch to separate.
#[rule]
fn utilization_zero_when_supplied_zero(e: Env, borrowed: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));

    let util = utilization(&e, Ray::from(borrowed), Ray::ZERO);
    cvlr_assert!(util.raw() == 0);
}

/// Native half of `utilization_bounded_when_borrowed_lte_supplied`.
///
/// `utilization` is `borrowed.div(supplied)` = `mul_div_half_up(borrowed, RAY,
/// supplied)`, whose native branch condition is `borrowed * RAY + supplied / 2
/// <= i128::MAX`, i.e. `borrowed <= (i128::MAX - supplied / 2) / RAY`.
///
/// This lemma is the half that runs on the inlined compiler-builtins limb code
/// (`__multi3`, `__udivti3`), which `docs/explanation/certora-sunbeam-prover-tuning.md`
/// §9.7 identifies as the likely source of the counterexample the whole rule
/// reported on 2026-09-02. Splitting the branch is what lets the next run say
/// which half produced it.
#[rule]
fn utilization_bounded_when_borrowed_lte_supplied_native(e: Env, borrowed: i128, supplied: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((1..=100 * RAY).contains(&supplied));
    cvlr_assume!(borrowed <= supplied);
    cvlr_assume!(borrowed <= (i128::MAX - supplied / 2) / RAY);

    let util = utilization(&e, Ray::from(borrowed), Ray::from(supplied));
    // Witness values for the call trace: the 2026-09-02 local counterexample
    // on the unsplit rule was reported without them.
    let util_raw = util.raw();
    clog!(borrowed);
    clog!(supplied);
    clog!(util_raw);
    cvlr_assert!(util.raw() >= 0);
    cvlr_assert!(util.raw() <= RAY);
}

/// Widened half of `utilization_bounded_when_borrowed_lte_supplied`: the biased
/// product leaves `i128`, so the division runs as exact `I256` host calls. The
/// bound is the exact complement of the native lemma's, so the pair covers the
/// original domain.
#[rule]
fn utilization_bounded_when_borrowed_lte_supplied_widened(e: Env, borrowed: i128, supplied: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&borrowed));
    cvlr_assume!((1..=100 * RAY).contains(&supplied));
    cvlr_assume!(borrowed <= supplied);
    cvlr_assume!(borrowed > (i128::MAX - supplied / 2) / RAY);

    let util = utilization(&e, Ray::from(borrowed), Ray::from(supplied));
    // Witness values for the call trace: the 2026-09-02 local counterexample
    // on the unsplit rule was reported without them.
    let util_raw = util.raw();
    clog!(borrowed);
    clog!(supplied);
    clog!(util_raw);
    cvlr_assert!(util.raw() >= 0);
    cvlr_assert!(util.raw() <= RAY);
}

/// No lemma split: zero utilization returns before the multiply.
#[rule]
fn deposit_rate_zero_when_no_utilization(e: Env, borrow_rate: i128, reserve_bps: u32) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));
    cvlr_assume!(reserve_bps < BPS as u32);

    let rate = calculate_deposit_rate(
        &e,
        Ray::ZERO,
        Ray::from(borrow_rate),
        Bps::from(i128::from(reserve_bps)),
    );
    cvlr_assert!(rate.raw() == 0);
}

/// Native half of `deposit_rate_not_above_borrow_rate`.
///
/// `calculate_deposit_rate` has one branch-relevant product,
/// `utilization.mul(borrow_rate)` = `mul_div_half_up(util_raw, borrow_rate,
/// RAY)`; the `apply_to_ray` that follows multiplies a value below `2 RAY` by
/// at most `BPS`, so it never leaves the native path and needs no bound of its
/// own. `borrow_rate.max(1)` keeps the divisor total: at `borrow_rate == 0` the
/// product is zero and native for every `util_raw`, and the bound then admits
/// the whole utilization range.
#[rule]
fn deposit_rate_not_above_borrow_rate_native(
    e: Env,
    util_raw: i128,
    borrow_rate: i128,
    reserve_bps: u32,
) {
    cvlr_assume!((0..=RAY).contains(&util_raw));
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));
    cvlr_assume!(reserve_bps < BPS as u32);
    cvlr_assume!(util_raw <= (i128::MAX - RAY / 2) / borrow_rate.max(1));

    let rate = calculate_deposit_rate(
        &e,
        Ray::from(util_raw),
        Ray::from(borrow_rate),
        Bps::from(i128::from(reserve_bps)),
    );
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= borrow_rate);
}

/// Widened half of `deposit_rate_not_above_borrow_rate`: `util_raw *
/// borrow_rate` overflows `i128`, so the rate-times-utilization step runs as
/// exact `I256` host calls. Exact complement of the native lemma's bound.
#[rule]
fn deposit_rate_not_above_borrow_rate_widened(
    e: Env,
    util_raw: i128,
    borrow_rate: i128,
    reserve_bps: u32,
) {
    cvlr_assume!((0..=RAY).contains(&util_raw));
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&borrow_rate));
    cvlr_assume!(reserve_bps < BPS as u32);
    cvlr_assume!(util_raw > (i128::MAX - RAY / 2) / borrow_rate.max(1));

    let rate = calculate_deposit_rate(
        &e,
        Ray::from(util_raw),
        Ray::from(borrow_rate),
        Bps::from(i128::from(reserve_bps)),
    );
    cvlr_assert!(rate.raw() >= 0);
    cvlr_assert!(rate.raw() <= borrow_rate);
}

/// No lemma split: `delta_ms == 0` returns `Ray::ONE` before the Taylor series,
/// so no multiply-divide runs.
#[rule]
fn compound_interest_identity_at_zero_delta(e: Env, rate: i128) {
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&rate));

    let factor = compound_interest(&e, Ray::from(rate), 0);
    cvlr_assert!(factor.raw() == RAY);
}

/// No lemma split: `old_index >= RAY` and `factor >= RAY` put the product at
/// `1e54` or more, so `mul_div_half_up` always widens to `I256` here. The
/// native branch is unreachable on this domain.
#[rule]
fn update_borrow_index_monotonic_when_factor_gte_one(e: Env, old_index: i128, factor: i128) {
    // The factor ceiling is 8 ray, not the 2 ray of `MAX_BORROW_RATE_RAY`, so
    // this lemma also covers the domain the deleted controller copy carried.
    cvlr_assume!((RAY..=10 * RAY).contains(&old_index));
    cvlr_assume!((RAY..=8 * RAY).contains(&factor));

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
    cvlr_assert!(out.raw() >= old_index);
}

/// No lemma split: `update_supply_index` runs two multiply-divides whose branch
/// conditions are not jointly determined by any single input bound —
/// `supplied.mul(old_index)` turns on `supplied * old_index`, and the
/// `mul_div_floor_saturating` that follows turns on `new_value * RAY`, where
/// `new_value` is derived from the first result plus `rewards`. A two-way split
/// would leave both lemmas carrying a branch, so it buys nothing.
#[rule]
fn update_supply_index_monotonic_when_rewards_positive(
    e: Env,
    supplied: i128,
    old_index: i128,
    rewards: i128,
) {
    // `supplied == 0` is included: `update_supply_index` returns `old_index`
    // before any arithmetic there, and covering it lets this lemma replace the
    // deleted controller copy outright.
    cvlr_assume!((0..=100 * RAY).contains(&supplied));
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

/// No lemma split here: `accrued_interest_split_is_conservative`
/// (`rate_index_accounting_rules.rs`) proves the same identity and carries the
/// native/widened lemma pair. Keeping one un-split statement of the property is
/// the strong form; if this rule times out, split it on `borrowed * new_index`
/// the same way.
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

/// No lemma split: `last_timestamp == current_timestamp` makes
/// `simulate_update_indexes_body` return the stored indexes before any
/// compounding, so no multiply-divide runs.
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
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&supply_index));

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

/// The supply index never grows past `MAX_SUPPLY_INDEX_RAY` and never falls
/// below the caller's own (capped) starting index.
///
/// `rewards` was previously unbounded above. It is now capped at
/// `MAX_RAY_VALUE`, the documented ray-value ceiling: `rewards` is an accrued
/// interest amount, so it is bounded by the market's total value, which
/// `numeric-bounds.md` §3 bounds at 1e11 whole tokens. The bound excludes
/// reward amounts no market can hold.
///
/// Note the residual hidden bound this rule keeps: `update_supply_index`
/// computes `supplied.mul(old_index)`, which panics with `MathOverflow` once
/// `supplied * old_index / RAY` leaves `i128`. Sunbeam treats that panic as
/// `assume(false)`, so the upper corner of the `supplied x old_index` box is
/// pruned by the trap rather than by an assume. The assertion is proved on
/// whatever survives, which is the honest statement of the property.
#[rule]
fn update_supply_index_capped(e: Env, supplied: i128, old_index: i128, rewards: i128) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old_index));
    cvlr_assume!((0..=MAX_RAY_VALUE).contains(&rewards));

    let out = update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_index),
        Ray::from(rewards),
    );
    cvlr_assert!(out.raw() <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(out.raw() >= old_index.min(MAX_SUPPLY_INDEX_RAY));
}

/// No lemma split: `old_index >= RAY` and `factor >= RAY` again force the
/// widened path for every point in the domain.
#[rule]
fn update_borrow_index_capped(e: Env, old_index: i128, factor: i128) {
    cvlr_assume!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old_index));
    cvlr_assume!((RAY..=10 * RAY).contains(&factor));

    let out = update_borrow_index(&e, Ray::from(old_index), Ray::from(factor));
    cvlr_assert!(out.raw() <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(out.raw() >= RAY);
}

/// No lemma split: two independent products decide the branches —
/// `supplied * old_index` inside `update_supply_index` and
/// `old_index * rewards` in the bound this rule computes — so no single input
/// bound settles both.
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

/// Booking a protocol fee as supply-index shares can never push the supply
/// share total past `i128::MAX` (INV-POOL-03 headroom clause).
///
/// Every operand is bounded to the domain production can actually produce:
///
/// - `supply_index` to `[SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY]`. The
///   pool clamps the index at both ends on every write — `update_supply_index`
///   caps at `MAX_SUPPLY_INDEX_RAY` and `apply_bad_debt_to_supply_index` floors
///   at `SUPPLY_INDEX_FLOOR_RAW` — so an index outside the interval is
///   unreachable, not merely unlikely. The previous form left the index
///   unbounded above, which is what made this rule a nonlinear query over the
///   full `i128` range under `precise_bitwise_ops` (review F8).
/// - `fee` and `supplied` to `MAX_RAY_VALUE`. See that constant: 1e11 whole
///   tokens is the documented ray-value ceiling, ~200x above the largest
///   configured supply cap.
///
/// The saturating branch of `mul_div_floor_saturating` stays reachable inside
/// these bounds (`fee * RAY / SUPPLY_INDEX_FLOOR_RAW` is up to `1e41`), so the
/// headroom clamp this rule is about is still exercised.
#[rule]
fn protocol_fee_shares_bounded_by_headroom(e: Env, fee: i128, supply_index: i128, supplied: i128) {
    cvlr_assume!((0..=MAX_RAY_VALUE).contains(&fee));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&supply_index));
    cvlr_assume!((0..=MAX_RAY_VALUE).contains(&supplied));

    let out = protocol_fee_shares(
        &e,
        Ray::from(fee),
        Ray::from(supply_index),
        Ray::from(supplied),
    );
    cvlr_assert!(out.raw() >= 0);
    cvlr_assert!(out.raw() <= i128::MAX - supplied);
}

/// No lemma split: `fee * RAY` is at least `1e56` for every non-trivial point
/// of this domain (`fee <= 100 RAY`, and the native path needs
/// `fee <= 1.7e11`), so the interesting states are all widened. The rule is
/// kept whole because it compares two implementations and the comparison is
/// only meaningful when both take the same branch.
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
    let plain = mul_div_floor(&e, fee, RAY, supply_index);
    cvlr_assert!(out.raw() == plain);
    cvlr_assert!(mul_div_floor(&e, out.raw(), supply_index, RAY) <= fee);
}

#[rule]
fn rates_reachability(e: Env, asset: Address) {
    let params = valid_params(asset);
    let rate = calculate_borrow_rate(&e, Ray::from(RAY / 2), &params);
    cvlr_satisfy!(rate.raw() > 0);
}
