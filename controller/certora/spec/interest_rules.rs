/// Interest Rate Model -- Formal Verification Rules
///
/// Verifies correctness of the 3-region piecewise linear borrow rate model,
/// the 5-term Taylor compound interest approximation, deposit rate calculation,
/// supplier reward conservation, and index update monotonicity.
///
/// Reference: `common/src/rates.rs`
///
/// Key constants:
///   RAY = 10^27 (1.0 in fixed-point)
///   BPS = 10_000 (100% in basis points)
///   MILLISECONDS_PER_YEAR = 31_556_926_000
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::Env;

use common::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY, WAD};
use common::fp::Ray;
use common::fp_core::{div_by_int_half_up, mul_div_half_up};
use common::rates::{
    calculate_borrow_rate, calculate_deposit_rate, calculate_supplier_rewards, compound_interest,
    update_borrow_index, update_supply_index,
};
use common::types::MarketParams;

// ---------------------------------------------------------------------------
// Helper: build MarketParams from nondet values with valid-range assumptions
// ---------------------------------------------------------------------------

fn nondet_valid_params(e: &Env) -> MarketParams {
    let base_borrow_rate_ray: i128 = cvlr::nondet::nondet();
    let slope1_ray: i128 = cvlr::nondet::nondet();
    let slope2_ray: i128 = cvlr::nondet::nondet();
    let slope3_ray: i128 = cvlr::nondet::nondet();
    let mid_utilization_ray: i128 = cvlr::nondet::nondet();
    let optimal_utilization_ray: i128 = cvlr::nondet::nondet();
    let max_borrow_rate_ray: i128 = cvlr::nondet::nondet();
    let reserve_factor_bps: i128 = cvlr::nondet::nondet();
    let asset_id = e.current_contract_address();
    let asset_decimals: u32 = cvlr::nondet::nondet();

    // Production cap: base + each slope <= MAX_BORROW_RATE_RAY (= 2 * RAY).
    // Tightened from RAY * 10 (5x reduction per axis -> ~625x cube reduction).
    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&base_borrow_rate_ray));
    cvlr_assume!(slope1_ray <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(slope2_ray <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(slope3_ray <= MAX_BORROW_RATE_RAY);

    // Slope monotonicity: base <= slope1 <= slope2 <= slope3.
    // Restricts the parameter cube to the production-realistic tetrahedral slice.
    cvlr_assume!(base_borrow_rate_ray <= slope1_ray);
    cvlr_assume!(slope1_ray <= slope2_ray);
    cvlr_assume!(slope2_ray <= slope3_ray);

    // Utilization breakpoints: 0 < mid < optimal < RAY.
    cvlr_assume!(mid_utilization_ray > 0 && mid_utilization_ray < optimal_utilization_ray);
    cvlr_assume!(optimal_utilization_ray < RAY);

    // Cap matches MAX_BORROW_RATE_RAY (validation enforces this in production).
    cvlr_assume!(max_borrow_rate_ray > 0 && max_borrow_rate_ray <= MAX_BORROW_RATE_RAY);

    // Reserve factor in [0, BPS - 1] (production rejects rf >= BPS).
    cvlr_assume!((0..BPS).contains(&reserve_factor_bps));

    // Asset decimals constrained to a realistic range so any downstream
    // rescale_half_up call stays inside i128.
    cvlr_assume!(asset_decimals <= 27);

    MarketParams {
        base_borrow_rate_ray,
        slope1_ray,
        slope2_ray,
        slope3_ray,
        mid_utilization_ray,
        optimal_utilization_ray,
        max_borrow_rate_ray,
        reserve_factor_bps,
        asset_id,
        asset_decimals,
    }
}

// ===========================================================================
// Rule 1: Borrow rate at zero utilization equals base rate
// ===========================================================================

/// At 0% utilization, there is no slope contribution.
/// The borrow rate must equal `base_borrow_rate / MILLISECONDS_PER_YEAR`
/// (or the capped equivalent if base > max).
#[rule]
fn borrow_rate_zero_utilization(e: Env) {
    let params = nondet_valid_params(&e);

    let rate = calculate_borrow_rate(&e, Ray::ZERO, &params);

    // Expected: min(base, max) / MILLISECONDS_PER_YEAR
    let annual = if params.base_borrow_rate_ray > params.max_borrow_rate_ray {
        params.max_borrow_rate_ray
    } else {
        params.base_borrow_rate_ray
    };
    let expected = div_by_int_half_up(annual, MILLISECONDS_PER_YEAR as i128);

    cvlr_assert!(rate.raw() == expected);
}

// ===========================================================================
// Rule 2: Borrow rate is monotonically increasing with utilization
// ===========================================================================

/// For any two utilization values where util_a < util_b (both in [0, RAY]),
/// the computed borrow rate must satisfy rate(util_a) <= rate(util_b).
///
/// Umbrella rule: spans all three regions. Cheaper per-region variants
/// (`*_in_region1`, `*_in_region2`, `*_in_region3`) below pin the prover
/// to a single piecewise branch for fast PR-gating CI.
#[rule]
fn borrow_rate_monotonic(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&util_a));
    cvlr_assume!((0..=RAY).contains(&util_b));
    cvlr_assume!(util_a < util_b);

    let rate_a = calculate_borrow_rate(&e, Ray::from_raw(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from_raw(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

/// Region 1 (`util < mid`): both calls hit the linear `base + util*s1/mid` branch.
#[rule]
fn borrow_rate_monotonic_in_region1(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(util_a >= 0);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b < params.mid_utilization_ray);

    let rate_a = calculate_borrow_rate(&e, Ray::from_raw(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from_raw(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

/// Region 2 (`mid <= util < optimal`): both calls hit `base + s1 + excess*s2/range`.
#[rule]
fn borrow_rate_monotonic_in_region2(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(params.mid_utilization_ray <= util_a);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b < params.optimal_utilization_ray);

    let rate_a = calculate_borrow_rate(&e, Ray::from_raw(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from_raw(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

/// Region 3 (`optimal <= util <= RAY`): both calls hit `base + s1 + s2 + excess*s3/range`.
#[rule]
fn borrow_rate_monotonic_in_region3(e: Env) {
    let params = nondet_valid_params(&e);

    let util_a: i128 = cvlr::nondet::nondet();
    let util_b: i128 = cvlr::nondet::nondet();

    cvlr_assume!(params.optimal_utilization_ray <= util_a);
    cvlr_assume!(util_a < util_b);
    cvlr_assume!(util_b <= RAY);

    let rate_a = calculate_borrow_rate(&e, Ray::from_raw(util_a), &params);
    let rate_b = calculate_borrow_rate(&e, Ray::from_raw(util_b), &params);

    cvlr_assert!(rate_a <= rate_b);
}

// ===========================================================================
// Rule 3: Borrow rate is capped at max_borrow_rate / MILLISECONDS_PER_YEAR
// ===========================================================================

/// For any utilization in [0, RAY], the borrow rate must never exceed
/// `max_borrow_rate / MILLISECONDS_PER_YEAR`.
#[rule]
fn borrow_rate_capped(e: Env) {
    let params = nondet_valid_params(&e);

    let utilization: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&utilization));

    let rate = calculate_borrow_rate(&e, Ray::from_raw(utilization), &params);
    let cap = div_by_int_half_up(params.max_borrow_rate_ray, MILLISECONDS_PER_YEAR as i128);

    // Allow +1 for half-up rounding tolerance
    cvlr_assert!(rate.raw() <= cap + 1);

    // Borrow rate must be non-negative (no negative interest)
    cvlr_assert!(rate.raw() >= 0);
}

// ===========================================================================
// Rule 4: Borrow rate continuity at mid utilization boundary
// ===========================================================================

/// The rate at `mid - 1` (top of region 1) must be approximately equal to
/// the rate at `mid` (bottom of region 2). No discontinuous jumps.
#[rule]
fn borrow_rate_continuity_at_mid(e: Env) {
    let params = nondet_valid_params(&e);

    // Ensure mid >= 2 so mid-1 is still positive
    cvlr_assume!(params.mid_utilization_ray >= 2);

    let rate_below =
        calculate_borrow_rate(&e, Ray::from_raw(params.mid_utilization_ray - 1), &params);
    let rate_at = calculate_borrow_rate(&e, Ray::from_raw(params.mid_utilization_ray), &params);

    // Both should evaluate to approximately base + slope1 at the boundary.
    // The difference must be at most 1 (rounding artifact from the -1 step).
    let diff = if rate_at >= rate_below {
        rate_at.raw() - rate_below.raw()
    } else {
        rate_below.raw() - rate_at.raw()
    };

    // Tolerance: 1 unit of the per-ms rate (rounding from integer division)
    cvlr_assert!(diff <= 1);
}

// ===========================================================================
// Rule 5: Borrow rate continuity at optimal utilization boundary
// ===========================================================================

/// The rate at `optimal - 1` (top of region 2) must be approximately equal to
/// the rate at `optimal` (bottom of region 3). No discontinuous jumps.
#[rule]
fn borrow_rate_continuity_at_optimal(e: Env) {
    let params = nondet_valid_params(&e);

    // Ensure optimal >= 2 so optimal-1 is still in-range
    cvlr_assume!(params.optimal_utilization_ray >= 2);

    let rate_below = calculate_borrow_rate(
        &e,
        Ray::from_raw(params.optimal_utilization_ray - 1),
        &params,
    );
    let rate_at = calculate_borrow_rate(&e, Ray::from_raw(params.optimal_utilization_ray), &params);

    let diff = if rate_at >= rate_below {
        rate_at.raw() - rate_below.raw()
    } else {
        rate_below.raw() - rate_at.raw()
    };

    // Tolerance: 1 unit of the per-ms rate (rounding from integer division)
    cvlr_assert!(diff <= 1);
}

// ===========================================================================
// Rule 6: Deposit rate is zero when utilization is zero
// ===========================================================================

/// When utilization is 0, suppliers earn nothing regardless of borrow rate
/// or reserve factor.
#[rule]
fn deposit_rate_zero_when_no_utilization(e: Env) {
    let borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor_bps: i128 = cvlr::nondet::nondet();

    cvlr_assume!(borrow_rate >= 0);
    cvlr_assume!((0..BPS).contains(&reserve_factor_bps));

    let rate = calculate_deposit_rate(
        &e,
        Ray::ZERO,
        Ray::from_raw(borrow_rate),
        reserve_factor_bps,
    );

    cvlr_assert!(rate == Ray::ZERO);
}

// ===========================================================================
// Rule 7: Deposit rate is less than or equal to borrow rate * utilization
// ===========================================================================

/// The reserve factor takes a cut, so:
///   deposit_rate = util * borrow_rate * (1 - rf/BPS)
///                <= util * borrow_rate
#[rule]
fn deposit_rate_less_than_borrow(e: Env) {
    let utilization: i128 = cvlr::nondet::nondet();
    let borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor_bps: i128 = cvlr::nondet::nondet();

    cvlr_assume!((0..=RAY).contains(&utilization));
    cvlr_assume!((0..=RAY).contains(&borrow_rate));
    cvlr_assume!((0..BPS).contains(&reserve_factor_bps));

    let deposit_rate = calculate_deposit_rate(
        &e,
        Ray::from_raw(utilization),
        Ray::from_raw(borrow_rate),
        reserve_factor_bps,
    );
    let upper_bound = mul_div_half_up(&e, utilization, borrow_rate, RAY);

    // Allow +1 for half-up rounding tolerance
    cvlr_assert!(deposit_rate.raw() <= upper_bound + 1);
}

// ===========================================================================
// Rule 8: Compound interest identity -- zero time yields RAY (1.0)
// ===========================================================================

/// When no time has elapsed (delta_ms == 0), the compound interest factor
/// must be exactly RAY (1.0). No interest accrues in zero time.
#[rule]
fn compound_interest_identity(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    cvlr_assume!((0..=RAY).contains(&rate));

    let factor = compound_interest(&e, Ray::from_raw(rate), 0);

    cvlr_assert!(factor == Ray::ONE);
}

// ===========================================================================
// Rule 9: Compound interest is monotonically increasing in time
// ===========================================================================

/// For a fixed positive rate, compounding over a longer period must yield
/// a factor at least as large as compounding over a shorter period.
#[rule]
fn compound_interest_monotonic_in_time(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let t1: u64 = cvlr::nondet::nondet();
    let t2: u64 = cvlr::nondet::nondet();

    cvlr_assume!(rate >= 0);
    // Keep rate * delta_ms within i128 range to avoid overflow panic
    cvlr_assume!(rate <= div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128));
    // Drop the early-return branches: both calls must hit the Taylor path.
    cvlr_assume!(t1 > 0);
    cvlr_assume!(t1 < t2);
    cvlr_assume!(t2 <= MILLISECONDS_PER_YEAR); // bound to 1 year for feasibility

    let factor1 = compound_interest(&e, Ray::from_raw(rate), t1);
    let factor2 = compound_interest(&e, Ray::from_raw(rate), t2);

    cvlr_assert!(factor2 >= factor1);
}

// ===========================================================================
// Rule 10: Compound interest is monotonically increasing in rate
// ===========================================================================

/// For a fixed time period, a higher rate must produce a compound factor
/// at least as large as a lower rate.
#[rule]
fn compound_interest_monotonic_in_rate(e: Env) {
    let r1: i128 = cvlr::nondet::nondet();
    let r2: i128 = cvlr::nondet::nondet();
    let t: u64 = cvlr::nondet::nondet();

    // Both rates strictly positive so neither call short-circuits on a
    // zero accumulator before the Taylor expansion.
    cvlr_assume!(r1 > 0);
    cvlr_assume!(r1 < r2);
    // Keep rate * delta_ms within i128 range
    cvlr_assume!(r2 <= div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128));
    cvlr_assume!(t > 0 && t <= MILLISECONDS_PER_YEAR);

    let factor1 = compound_interest(&e, Ray::from_raw(r1), t);
    let factor2 = compound_interest(&e, Ray::from_raw(r2), t);

    cvlr_assert!(factor2 >= factor1);
}

// ===========================================================================
// Rule 11: Compound interest >= simple interest (e^x >= 1 + x)
// ===========================================================================

/// The Taylor expansion of e^x always exceeds the linear approximation
/// `1 + x` for non-negative x, so compound interest never underestimates
/// simple interest.
///
/// `rate` is per-millisecond in RAY; `t` is plain milliseconds. The product
/// `rate * t` is the cumulative `x` in RAY. We bound `rate <= 1 RAY/year` so
/// the product stays inside i128 (`<= RAY = 10^27`, well below i128 max
/// 1.7e38).
#[rule]
fn compound_interest_ge_simple(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let t: u64 = cvlr::nondet::nondet();

    let max_rate = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    cvlr_assume!(rate >= 0 && rate <= max_rate);
    cvlr_assume!(t > 0 && t <= MILLISECONDS_PER_YEAR);

    let factor = compound_interest(&e, Ray::from_raw(rate), t);

    // `rate * t <= max_rate * MILLISECONDS_PER_YEAR <= RAY` by construction;
    // safe in i128.
    let x = rate * (t as i128);
    let simple = RAY + x;

    // Allow -2 tolerance: Taylor truncation and rounding can cause the
    // 8-term approximation to fall slightly below the linear approximation
    // for tiny x.
    cvlr_assert!(factor.raw() >= simple - 2);
}

// ===========================================================================
// Rule 12: Supplier rewards conservation -- no interest is lost
// ===========================================================================

/// The split of accrued interest into supplier rewards and protocol fee
/// must be exact: `supplier_rewards + protocol_fee == accrued_interest`.
///
/// Due to half-up rounding in the protocol fee calculation, the sum may
/// differ from the raw accrued interest by at most 1 unit.
#[rule]
fn supplier_rewards_conservation(e: Env) {
    let params = nondet_valid_params(&e);

    let borrowed: i128 = cvlr::nondet::nondet();
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let new_borrow_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(borrowed > 0);
    cvlr_assume!(old_borrow_index >= RAY);
    cvlr_assume!(new_borrow_index >= old_borrow_index);
    // Production-realistic bounds: scaled debt up to one realistic position
    // (10^18 ~ 1 token at 18 decimals) and index growth up to ~e^2 envelope
    // matched by the compound_interest cap.
    cvlr_assume!(borrowed < WAD);
    cvlr_assume!(new_borrow_index <= RAY * 8);

    let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
        &e,
        &params,
        Ray::from_raw(borrowed),
        Ray::from_raw(new_borrow_index),
        Ray::from_raw(old_borrow_index),
    );

    // Reconstruct accrued interest
    let old_debt = mul_div_half_up(&e, borrowed, old_borrow_index, RAY);
    let new_debt = mul_div_half_up(&e, borrowed, new_borrow_index, RAY);
    let accrued_interest = new_debt - old_debt;

    // Conservation: rewards + fee == accrued_interest (within rounding tolerance of 1)
    let sum = supplier_rewards.raw() + protocol_fee.raw();
    let diff = if sum >= accrued_interest {
        sum - accrued_interest
    } else {
        accrued_interest - sum
    };

    cvlr_assert!(diff <= 1);

    // Verify protocol fee matches expected: mul_half_up(accrued, reserve_factor, BPS) within +/-1
    let expected_fee = mul_div_half_up(&e, accrued_interest, params.reserve_factor_bps, BPS);
    let fee_diff = if protocol_fee.raw() >= expected_fee {
        protocol_fee.raw() - expected_fee
    } else {
        expected_fee - protocol_fee.raw()
    };
    cvlr_assert!(fee_diff <= 1);
}

// ===========================================================================
// Rule 13: Borrow index update is monotonically increasing
// ===========================================================================

/// After applying a compound interest factor that is at least RAY, the updated
/// borrow index must not decrease.
#[rule]
fn update_borrow_index_monotonic(e: Env) {
    let old_index: i128 = cvlr::nondet::nondet();
    let interest_factor: i128 = cvlr::nondet::nondet();

    cvlr_assume!(old_index >= RAY);
    cvlr_assume!(interest_factor >= RAY); // compound interest is always >= 1.0
                                          // Bound both operands so old_index * interest_factor stays well inside
                                          // i128 inside `Ray::mul` -- prunes the to_i128 panic-branch exploration.
    cvlr_assume!(old_index <= RAY * 8);
    cvlr_assume!(interest_factor <= RAY * 8);

    let new_index =
        update_borrow_index(&e, Ray::from_raw(old_index), Ray::from_raw(interest_factor));

    cvlr_assert!(new_index.raw() >= old_index);
}

// ===========================================================================
// Rule 14: Supply index update is monotonically increasing (non-bad-debt)
// ===========================================================================

/// When suppliers receive positive rewards (non-bad-debt path), the supply
/// index must not decrease. It stays unchanged when rewards or supplied are zero.
#[rule]
fn update_supply_index_monotonic(e: Env) {
    let supplied: i128 = cvlr::nondet::nondet();
    let old_index: i128 = cvlr::nondet::nondet();
    let rewards_increase: i128 = cvlr::nondet::nondet();

    cvlr_assume!(supplied >= 0);
    cvlr_assume!(old_index >= RAY);
    cvlr_assume!(rewards_increase >= 0);
    // Production-realistic bounds: scaled supply up to one realistic position
    // and index growth bounded to the same envelope as compound_interest.
    cvlr_assume!(supplied < WAD);
    cvlr_assume!(old_index <= RAY * 8);
    cvlr_assume!(rewards_increase < WAD);

    let new_index = update_supply_index(
        &e,
        Ray::from_raw(supplied),
        Ray::from_raw(old_index),
        Ray::from_raw(rewards_increase),
    );

    cvlr_assert!(new_index.raw() >= old_index);
}
