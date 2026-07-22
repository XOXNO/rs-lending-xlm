//! Primary vs anchor tolerance: blend within the band, otherwise revert.

use common::constants::{BPS_DECIMALS, RAY, RAY_DECIMALS};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OracleTolerance;
use soroban_sdk::{panic_with_error, Env};

/// Final Wad price from a required primary/anchor pair: the midpoint when the
/// two agree within the tolerance band, otherwise reverts `UnsafePriceNotAllowed`.
/// Both feeds are required upstream, so the prices are concrete.
pub(crate) fn midpoint_if_in_band(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OracleTolerance,
) -> i128 {
    if !within_tolerance_band(env, anchor, primary, tolerance) {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    midpoint_price(env, anchor, primary)
}

/// True when primary/anchor agree within the inclusive BPS band.
pub(crate) fn within_tolerance_band(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OracleTolerance,
) -> bool {
    // dimensional: anchor/primary are same-asset Wad<Price(USD/asset)>.
    anchor_ratio_bps(env, anchor, primary)
        .is_some_and(|r| ratio_in_band(r, tolerance.upper_ratio_bps, tolerance.lower_ratio_bps))
}

/// Integer midpoint, or `0` on overflow (view-safe).
pub(crate) fn midpoint_price_or_zero(env: &Env, anchor_price: i128, primary_price: i128) -> i128 {
    midpoint_price_checked(env, anchor_price, primary_price).unwrap_or(0)
}

fn midpoint_price(env: &Env, anchor_price: i128, primary_price: i128) -> i128 {
    midpoint_price_checked(env, anchor_price, primary_price)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

fn midpoint_price_checked(env: &Env, anchor_price: i128, primary_price: i128) -> Option<i128> {
    let _ = env;
    anchor_price.checked_add(primary_price).map(|sum| sum / 2)
}

/// Primary/anchor ratio in BPS. `None` when the pair is out of band by
/// construction: a zero anchor (undefined ratio) or a ratio that would overflow
/// the fixed-point narrowing.
fn anchor_ratio_bps(env: &Env, anchor: i128, primary: i128) -> Option<i128> {
    if anchor == 0 {
        return None;
    }
    // Ratio beyond any u32 BPS band is out of band.
    if primary / anchor > i128::from(u32::MAX) {
        return None;
    }
    // dimensional: primary / anchor is dimensionless; RAY is D27<1>, BPS is D4<1>.
    let ratio_ray = fp_core::mul_div_half_up(env, primary, RAY, anchor);
    Some(fp_core::rescale_half_up(
        ratio_ray,
        RAY_DECIMALS,
        BPS_DECIMALS,
    ))
}

/// True when a BPS ratio sits within the inclusive `[lower, upper]` band.
fn ratio_in_band(ratio_bps: i128, upper_bound_ratio: u32, lower_bound_ratio: u32) -> bool {
    ratio_bps <= i128::from(upper_bound_ratio) && ratio_bps >= i128::from(lower_bound_ratio)
}

#[cfg(test)]
#[path = "../tests/oracle/tolerance.rs"]
mod tests;
