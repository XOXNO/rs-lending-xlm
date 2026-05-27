//! Primary vs Anchor tolerance logic.
//!
//! This module implements the deviation checks and final price selection
//! between the primary (safe) and anchor (aggregator) sources according to
//! the configured tolerance bands.
//!
//! It is deliberately pure (no oracle client knowledge) so it can be
//! heavily summarized in Certora without affecting the rest of price resolution.

use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE, RAY,
};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OraclePriceFluctuation;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::ControllerCache;
use crate::validation;

pub(crate) fn calculate_final_price(
    cache: &ControllerCache,
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
                assert_with_error!(
                    env,
                    cache.oracle_policy.allows_unsafe_deviation(),
                    OracleError::UnsafePriceNotAllowed
                );
                // Use aggregator on high deviation.
                if cache.oracle_policy.prefers_aggregator_on_deviation() {
                    agg_price
                } else {
                    safe_price
                }
            }
        }
        (Some(agg_price), None) => agg_price,
        (None, Some(safe_price)) => safe_price,
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

// Checks if ratio is within bounds.
pub(crate) fn is_within_anchor(
    env: &Env,
    aggregator: i128,
    safe: i128,
    upper_bound_ratio: u32,
    lower_bound_ratio: u32,
) -> bool {
    if aggregator == 0 {
        return false;
    }
    // safe and aggregator are same-scale i128 prices (Wad). Their ratio at
    // RAY precision = `safe * RAY / aggregator`; rescale RAY→BPS for comparison.
    let ratio_ray = fp_core::mul_div_half_up(env, safe, RAY, aggregator);
    let ratio_bps = fp_core::rescale_half_up(ratio_ray, 27, 4);
    let upper = i128::from(upper_bound_ratio);
    let lower = i128::from(lower_bound_ratio);

    ratio_bps <= upper && ratio_bps >= lower
}

/// i128 to u32 (checked). Used only at config time when building
/// `OraclePriceFluctuation` bands from owner/oracle-role inputs.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

/// Computes the upper/lower bounds for a single tolerance value in BPS.
pub(crate) fn calculate_tolerance_range(env: &Env, tolerance: i128) -> (i128, i128) {
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

/// Validates the first/last tolerance inputs against constants and
/// `validate_oracle_bounds`, then constructs the four ratio BPS fields
/// for `OraclePriceFluctuation`.
///
/// This is the single place that turns raw BPS tolerance inputs (from
/// `configure_market_oracle` / `edit_oracle_tolerance`) into the
/// persisted band struct. Kept pure so it can live in the tolerance
/// module alongside the runtime decision logic.
pub(crate) fn validate_and_calculate_tolerances(
    env: &Env,
    first_tolerance: u32,
    last_tolerance: u32,
) -> OraclePriceFluctuation {
    let first = i128::from(first_tolerance);
    let last = i128::from(last_tolerance);
    assert_with_error!(
        env,
        (MIN_FIRST_TOLERANCE..=MAX_FIRST_TOLERANCE).contains(&first),
        OracleError::BadFirstTolerance
    );
    assert_with_error!(
        env,
        (MIN_LAST_TOLERANCE..=MAX_LAST_TOLERANCE).contains(&last),
        OracleError::BadLastTolerance
    );

    validation::validate_oracle_bounds(env, first, last);

    let (first_upper, first_lower) = calculate_tolerance_range(env, first);
    let (last_upper, last_lower) = calculate_tolerance_range(env, last);

    OraclePriceFluctuation {
        first_upper_ratio_bps: bps_i128_to_u32(env, first_upper),
        first_lower_ratio_bps: bps_i128_to_u32(env, first_lower),
        last_upper_ratio_bps: bps_i128_to_u32(env, last_upper),
        last_lower_ratio_bps: bps_i128_to_u32(env, last_lower),
    }
}
