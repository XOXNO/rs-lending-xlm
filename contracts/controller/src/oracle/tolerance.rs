//! Primary vs Anchor tolerance logic.
//!
//! Applies tolerance bands between primary and anchor prices.

use crate::constants::RAY;
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use controller_interface::types::OraclePriceFluctuation;
use soroban_sdk::{panic_with_error, Env};

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
                if cache.oracle_policy.requires_blended_first_band() {
                    midpoint_price(env, anchor_price, primary_price)
                } else {
                    primary_price
                }
            } else if is_within_anchor(
                env,
                anchor_price,
                primary_price,
                tolerance.last_upper_ratio_bps,
                tolerance.last_lower_ratio_bps,
            ) {
                midpoint_price(env, anchor_price, primary_price)
            } else {
                // Beyond the last band, only single-source fallback policies
                // tolerate this divergence. Other policies, including liquidation,
                // revert; fallback policies keep the primary price.
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

fn midpoint_price(env: &Env, anchor_price: i128, primary_price: i128) -> i128 {
    anchor_price
        .checked_add(primary_price)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
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
    // A primary/anchor ratio beyond any representable u32 BPS band is
    // out of band by definition.
    if primary / anchor > i128::from(u32::MAX) {
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

#[cfg(test)]
#[path = "../../tests/oracle/tolerance.rs"]
mod tests;
