//! Config-time tolerance-band validation and construction.
//!
//! Turns raw BPS inputs (`configure_market_oracle` / `edit_oracle_tolerance`)
//! into the persisted `OraclePriceFluctuation` band struct. The runtime
//! decision logic that consumes the bands lives in the controller.

use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE,
};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use controller_interface::types::OraclePriceFluctuation;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

/// i128 to u32 (checked). Used only at config time when building
/// `OraclePriceFluctuation` bands from owner/oracle-role inputs.
pub(crate) fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

/// Requires `last_tolerance_bps > first_tolerance_bps` (wider second band).
pub(crate) fn require_last_tolerance_gt_first(env: &Env, first: u32, last: u32) {
    assert_with_error!(env, last > first, OracleError::BadAnchorTolerances);
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

/// Validates the first/last tolerance inputs and builds the four ratio BPS
/// fields of `OraclePriceFluctuation`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn test_validate_and_calculate_tolerances_rejects_last_lte_first() {
        let env = Env::default();
        let _ = validate_and_calculate_tolerances(&env, MIN_FIRST_TOLERANCE, MIN_FIRST_TOLERANCE);
    }

    #[test]
    #[should_panic]
    fn test_validate_and_calculate_tolerances_rejects_first_below_min() {
        let env = Env::default();
        let _ =
            validate_and_calculate_tolerances(&env, MIN_FIRST_TOLERANCE - 1, MIN_LAST_TOLERANCE);
    }

    #[test]
    fn test_calculate_tolerance_range_scales_bounds() {
        let env = Env::default();
        let (upper, lower) = calculate_tolerance_range(&env, 200);
        assert!(upper > BPS);
        assert!(lower < BPS);
    }
}
