//! Primary vs Anchor tolerance logic.

use common::constants::{BPS_DECIMALS, RAY, RAY_DECIMALS};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use controller_interface::types::OraclePriceFluctuation;
use soroban_sdk::{panic_with_error, Env};

use crate::cache::Cache;

/// Final price plus the tolerance bands it was selected from, so callers do not
/// recompute `is_within_anchor` for the same primary/anchor pair.
pub(crate) struct FinalPrice {
    pub price_wad: i128,
    pub within_first: bool,
    pub within_second: bool,
}

pub(crate) fn calculate_final_price(
    cache: &Cache,
    anchor: Option<i128>,
    primary: Option<i128>,
    tolerance: &OraclePriceFluctuation,
) -> FinalPrice {
    let env = cache.env();
    match (anchor, primary) {
        (Some(anchor_price), Some(primary_price)) => {
            // dimensional: anchor/primary are same-asset Wad<Price(USD/asset)>.
            // Compute the primary/anchor ratio once, then test it against both bands.
            let ratio_bps = anchor_ratio_bps(env, anchor_price, primary_price);
            let within_first = ratio_bps.is_some_and(|r| {
                ratio_in_band(r, tolerance.first_upper_ratio_bps, tolerance.first_lower_ratio_bps)
            });
            // The last band is wider than the first (enforced at config time), so
            // `within_first` implies `within_second`.
            let within_second = ratio_bps.is_some_and(|r| {
                ratio_in_band(r, tolerance.last_upper_ratio_bps, tolerance.last_lower_ratio_bps)
            });
            let price_wad = if within_first {
                if cache.oracle_policy.requires_blended_first_band() {
                    midpoint_price(env, anchor_price, primary_price)
                } else {
                    primary_price
                }
            } else if within_second {
                midpoint_price(env, anchor_price, primary_price)
            } else {
                // Beyond the last band, only single-source fallback policies
                // tolerate this divergence. Other policies, including liquidation,
                // revert; fallback policies keep the primary price.
                if !cache.oracle_policy.allows_unsafe_deviation() {
                    panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
                }
                primary_price
            };
            FinalPrice {
                price_wad,
                within_first,
                within_second,
            }
        }
        (Some(anchor_price), None) => FinalPrice {
            price_wad: anchor_price,
            within_first: false,
            within_second: false,
        },
        (None, Some(primary_price)) => FinalPrice {
            price_wad: primary_price,
            within_first: false,
            within_second: false,
        },
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

/// Band predicate retained for the unit test (and mirrored by the certora
/// harness); the production path tests the ratio inline via `anchor_ratio_bps`
/// and `ratio_in_band`, computing it once per pair.
#[cfg(test)]
pub(crate) fn is_within_anchor(
    env: &Env,
    anchor: i128,
    primary: i128,
    upper_bound_ratio: u32,
    lower_bound_ratio: u32,
) -> bool {
    match anchor_ratio_bps(env, anchor, primary) {
        Some(ratio_bps) => ratio_in_band(ratio_bps, upper_bound_ratio, lower_bound_ratio),
        None => false,
    }
}

/// Primary/anchor ratio in BPS, computed once per pair. `None` when the pair is
/// out of band by construction: a zero anchor (undefined ratio) or a ratio that
/// would overflow the fixed-point narrowing.
fn anchor_ratio_bps(env: &Env, anchor: i128, primary: i128) -> Option<i128> {
    if anchor == 0 {
        return None;
    }
    // A primary/anchor ratio beyond any representable u32 BPS band is
    // out of band by definition.
    if primary / anchor > i128::from(u32::MAX) {
        return None;
    }
    // dimensional: primary / anchor is dimensionless; RAY is D27<1>, BPS is D4<1>.
    let ratio_ray = fp_core::mul_div_half_up(env, primary, RAY, anchor);
    Some(fp_core::rescale_half_up(ratio_ray, RAY_DECIMALS, BPS_DECIMALS))
}

/// True when a BPS ratio sits within the inclusive `[lower, upper]` band.
fn ratio_in_band(ratio_bps: i128, upper_bound_ratio: u32, lower_bound_ratio: u32) -> bool {
    ratio_bps <= i128::from(upper_bound_ratio) && ratio_bps >= i128::from(lower_bound_ratio)
}

#[cfg(test)]
#[path = "../../tests/oracle/tolerance.rs"]
mod tests;
