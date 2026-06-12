//! Primary vs Anchor tolerance logic.
//!
//! Applies tolerance bands between primary and anchor prices.

use crate::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE, RAY,
};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use controller_interface::types::OraclePriceFluctuation;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

pub(crate) fn calculate_final_price(
    cache: &Cache,
    anchor: Option<i128>,
    primary: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    let env = cache.env();
    match (anchor, primary) {
        (Some(anchor_price), Some(primary_price)) => {
            if is_within_anchor(
                env,
                anchor_price,
                primary_price,
                tolerance.first_upper_ratio_bps,
                tolerance.first_lower_ratio_bps,
            ) {
                primary_price
            } else if is_within_anchor(
                env,
                anchor_price,
                primary_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            ) {
                anchor_price
                    .checked_add(primary_price)
                    .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
                    / 2
            } else {
                // Beyond the last band: only single-source fallback policies tolerate
                // this divergence (all others, incl. liquidation, revert); keep the
                // primary price.
                if !cache.oracle_policy.allows_unsafe_deviation() {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                primary_price
            }
        }
        (Some(anchor_price), None) => anchor_price,
        (None, Some(primary_price)) => primary_price,
        (None, None) => {
            panic_with_error!(env, OracleError::NoLastPrice);
        }
    }
}

pub(crate) fn is_within_anchor(
    env: &Env,
    anchor: i128,
    primary: i128,
    upper_bound_ratio: u32,
    lower_bound_ratio: u32,
) -> bool {
    if anchor == 0 {
        return false;
    }
    // primary/anchor are same-scale (Wad) i128 prices; ratio = primary * RAY /
    // anchor at RAY precision, then rescaled RAY->BPS for comparison.
    let ratio_ray = fp_core::mul_div_half_up(env, primary, RAY, anchor);
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
/// fields of `OraclePriceFluctuation`. The single place that turns raw BPS
/// inputs (`configure_market_oracle` / `edit_oracle_tolerance`) into the
/// persisted band struct; kept pure to live beside the runtime decision logic.
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
    use crate::cache::Cache;
    use crate::oracle::policy::OraclePolicy;

    fn sample_tolerance() -> OraclePriceFluctuation {
        OraclePriceFluctuation {
            first_upper_ratio_bps: 10_200,
            first_lower_ratio_bps: 9_800,
            last_upper_ratio_bps: 10_500,
            last_lower_ratio_bps: 9_500,
        }
    }

    #[test]
    fn test_is_within_anchor_zero_anchor_returns_false() {
        let env = Env::default();
        assert!(!is_within_anchor(
            &env,
            0,
            100 * crate::constants::WAD,
            200,
            200
        ));
    }

    #[test]
    fn test_calculate_final_price_anchor_only() {
        let env = Env::default();
        let cache = Cache::build(&env, OraclePolicy::View);
        let price = calculate_final_price(&cache, Some(500), None, &sample_tolerance());
        assert_eq!(price, 500);
    }

    #[test]
    #[should_panic]
    fn test_calculate_final_price_none_none_panics() {
        let env = Env::default();
        let cache = Cache::build(&env, OraclePolicy::View);
        let _ = calculate_final_price(&cache, None, None, &sample_tolerance());
    }

    #[test]
    #[should_panic]
    fn test_calculate_final_price_unsafe_deviation_liquidation_panics() {
        let env = Env::default();
        let cache = Cache::build(&env, OraclePolicy::Liquidation);
        let tight = OraclePriceFluctuation {
            first_upper_ratio_bps: 10_010,
            first_lower_ratio_bps: 9_990,
            last_upper_ratio_bps: 10_020,
            last_lower_ratio_bps: 9_980,
        };
        let _ = calculate_final_price(
            &cache,
            Some(100 * crate::constants::WAD),
            Some(200 * crate::constants::WAD),
            &tight,
        );
    }

    #[test]
    fn test_calculate_final_price_unsafe_deviation_risk_decreasing_keeps_primary() {
        let env = Env::default();
        let cache = Cache::build(&env, OraclePolicy::RiskDecreasing);
        let tight = OraclePriceFluctuation {
            first_upper_ratio_bps: 10_010,
            first_lower_ratio_bps: 9_990,
            last_upper_ratio_bps: 10_020,
            last_lower_ratio_bps: 9_980,
        };
        let primary = 200 * crate::constants::WAD;
        let price = calculate_final_price(
            &cache,
            Some(100 * crate::constants::WAD),
            Some(primary),
            &tight,
        );
        assert_eq!(price, primary);
    }

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
