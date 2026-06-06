//! Certora harness for `controller::oracle::tolerance`.
//!
//! `calculate_final_price` and `is_within_anchor` perform non-trivial
// fixed-point ratio math that is expensive for the prover. This harness
// replaces the tolerance decision logic with a sound nondet bool while
// preserving the important control-flow branches and panic conditions.
//!
//! Lives separately so the clean production tolerance logic in `oracle/tolerance.rs`
// can remain untouched and well-documented.
//!
//! Part of the broader strategy to keep expensive math out of the prover
// while still exercising the policy branches that matter for rules.

use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE,
};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OraclePriceFluctuation;
use cvlr::nondet::nondet;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

pub(crate) fn calculate_final_price(
    cache: &Cache,
    aggregator: Option<i128>,
    safe: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    let env = cache.env();
    match (aggregator, safe) {
        (Some(agg_price), Some(safe_price)) => {
            if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.first_upper_ratio_bps,
                tolerance.first_lower_ratio_bps,
            ) {
                safe_price
            } else if is_within_anchor(
                env,
                agg_price,
                safe_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            ) {
                agg_price
                    .checked_add(safe_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else {
                if !cache.oracle_policy.allows_unsafe_deviation() {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                safe_price
            }
        }
        (Some(agg_price), None) => agg_price,
        (None, Some(safe_price)) => safe_price,
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

// Summary: tests whether `safe / aggregator` (in bps) sits inside
// `[lower_bound_ratio, upper_bound_ratio]`. The real implementation
// does an I256 ratio + BPS rescale that the prover can't traverse;
// returning a free nondet bool is sound because the boundary branch
// each call selects is determined by the inputs.
pub(crate) fn is_within_anchor(
    _env: &Env,
    aggregator: i128,
    _safe: i128,
    _upper_bound_ratio: u32,
    _lower_bound_ratio: u32,
) -> bool {
    // Preserve the `aggregator == 0` short-circuit from production:
    // the ratio is undefined, so production returns false.
    if aggregator == 0 {
        return false;
    }
    nondet()
}

/// i128 to u32 (checked). Used only at config time.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

pub(crate) fn require_last_tolerance_gt_first(env: &Env, first: u32, last: u32) {
    assert_with_error!(env, last > first, OracleError::BadAnchorTolerances);
}

pub(crate) fn calculate_tolerance_range(env: &Env, tolerance_bps: u32) -> (i128, i128) {
    let tolerance = i128::from(tolerance_bps);
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

pub(crate) fn validate_and_calculate_tolerances(
    env: &Env,
    first_tolerance: u32,
    last_tolerance: u32,
) -> OraclePriceFluctuation {
    assert_with_error!(
        env,
        (MIN_FIRST_TOLERANCE..=MAX_FIRST_TOLERANCE).contains(&first_tolerance),
        OracleError::BadFirstTolerance
    );
    assert_with_error!(
        env,
        (MIN_LAST_TOLERANCE..=MAX_LAST_TOLERANCE).contains(&last_tolerance),
        OracleError::BadLastTolerance
    );

    require_last_tolerance_gt_first(env, first_tolerance, last_tolerance);

    let (first_upper, first_lower) = calculate_tolerance_range(env, first_tolerance);
    let (last_upper, last_lower) = calculate_tolerance_range(env, last_tolerance);

    OraclePriceFluctuation {
        first_upper_ratio_bps: bps_i128_to_u32(env, first_upper),
        first_lower_ratio_bps: bps_i128_to_u32(env, first_lower),
        last_upper_ratio_bps: bps_i128_to_u32(env, last_upper),
        last_lower_ratio_bps: bps_i128_to_u32(env, last_lower),
    }
}
