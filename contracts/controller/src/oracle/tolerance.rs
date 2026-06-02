//! Primary vs Anchor tolerance logic.
//!
//! Deviation checks and final-price selection between the `primary` price and
//! the `anchor` price against the configured tolerance bands. The primary is
//! the value the protocol prices on; the anchor is the cross-check.
//!
//! Vocabulary note: the public price-component ABI
//! (`common::types` view fields) names these `safe_price_wad` and
//! `aggregator_price_wad` — i.e. `safe = primary`, `aggregator = anchor`. This
//! module (and the rest of the oracle code) uses `primary`/`anchor`
//! consistently; the view boundary in `oracle::mod::price_components` maps to
//! the ABI names.
//!
//! Deliberately pure (no oracle client knowledge) so Certora can summarize it
//! without affecting the rest of price resolution.

use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE, RAY,
};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OraclePriceFluctuation;
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;
use crate::validation;

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

/// Computes the upper/lower bounds for a single tolerance value in BPS.
pub(crate) fn calculate_tolerance_range(env: &Env, tolerance: i128) -> (i128, i128) {
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
