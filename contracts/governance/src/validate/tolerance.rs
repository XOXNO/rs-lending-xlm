//! Config-time tolerance-band validation and construction.
//!
//! Turns raw BPS inputs (`configure_market_oracle` / `edit_oracle_tolerance`)
//! into the persisted `OraclePriceFluctuation` band struct. The runtime
//! decision logic that consumes the bands lives in the controller.

use common::constants::{BPS, MAX_TOLERANCE, MIN_TOLERANCE};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use controller_interface::types::OraclePriceFluctuation;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

/// Checked i128-to-u32 conversion for tolerance band fields.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

/// Computes the upper/lower bounds for a single tolerance value in BPS.
pub(crate) fn calculate_tolerance_range(env: &Env, tolerance_bps: u32) -> (i128, i128) {
    let tolerance = i128::from(tolerance_bps);
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

/// Validates the tolerance input and builds the `OraclePriceFluctuation` band.
pub(crate) fn validate_and_calculate_tolerances(
    env: &Env,
    tolerance: u32,
) -> OraclePriceFluctuation {
    assert_with_error!(
        env,
        (MIN_TOLERANCE..=MAX_TOLERANCE).contains(&tolerance),
        OracleError::BadLastTolerance
    );

    let (upper, lower) = calculate_tolerance_range(env, tolerance);

    OraclePriceFluctuation {
        upper_ratio_bps: bps_i128_to_u32(env, upper),
        lower_ratio_bps: bps_i128_to_u32(env, lower),
    }
}

#[cfg(test)]
#[path = "../../tests/validate/tolerance.rs"]
mod tests;
