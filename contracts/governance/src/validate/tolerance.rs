//! Validation and derivation of oracle price tolerance bounds, expressed in
//! basis points.

use common::constants::{BPS, MAX_TOLERANCE, MIN_TOLERANCE};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OracleTolerance;

use soroban_sdk::{assert_with_error, panic_with_error, Env};

/// Converts a basis-point value from `i128` to `u32`. Panics with
/// `GenericError::MathOverflow` if `v` does not fit in `u32`.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

/// Computes the upper and lower bounds of the price ratio range implied by
/// `tolerance_bps`: the upper bound is `BPS + tolerance_bps`, and the lower
/// bound is `BPS * BPS / upper_bound`, rounded half up. Panics with
/// `GenericError::MathOverflow` if the upper bound addition overflows.
pub(crate) fn calculate_tolerance_range(env: &Env, tolerance_bps: u32) -> (i128, i128) {
    let tolerance = i128::from(tolerance_bps);
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

/// Panics with `OracleError::BadLastTolerance` unless `tolerance` falls within
/// `MIN_TOLERANCE..=MAX_TOLERANCE`, then returns the corresponding
/// `OracleTolerance` upper and lower ratio bounds in basis points.
pub(crate) fn validate_and_calculate_tolerances(env: &Env, tolerance: u32) -> OracleTolerance {
    assert_with_error!(
        env,
        (MIN_TOLERANCE..=MAX_TOLERANCE).contains(&tolerance),
        OracleError::BadLastTolerance
    );

    let (upper, lower) = calculate_tolerance_range(env, tolerance);

    OracleTolerance {
        upper_ratio_bps: bps_i128_to_u32(env, upper),
        lower_ratio_bps: bps_i128_to_u32(env, lower),
    }
}

#[cfg(test)]
#[path = "../../tests/validate/tolerance.rs"]
mod tests;
