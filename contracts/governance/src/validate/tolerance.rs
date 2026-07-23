//! Tolerance-BPS → `OracleTolerance` band for oracle config proposals.

use common::constants::{BPS, MAX_TOLERANCE, MIN_TOLERANCE};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OracleTolerance;

use soroban_sdk::{assert_with_error, panic_with_error, Env};

/// Checked i128-to-u32 conversion for tolerance band fields.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

pub(crate) fn calculate_tolerance_range(env: &Env, tolerance_bps: u32) -> (i128, i128) {
    let tolerance = i128::from(tolerance_bps);
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

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
