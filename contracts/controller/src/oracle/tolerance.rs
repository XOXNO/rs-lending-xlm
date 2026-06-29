//! Primary vs anchor tolerance: blend within the band, otherwise revert.

use common::constants::{BPS_DECIMALS, RAY, RAY_DECIMALS};
use common::errors::{GenericError, OracleError};
use common::math::fp_core;
use common::types::OraclePriceFluctuation;
use soroban_sdk::{panic_with_error, Env};

/// Final Wad price from a required primary/anchor pair: the midpoint when the
/// two agree within the tolerance band, otherwise reverts `UnsafePriceNotAllowed`.
/// Both feeds are required upstream, so the prices are concrete.
pub(crate) fn calculate_final_price(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OraclePriceFluctuation,
) -> i128 {
    // dimensional: anchor/primary are same-asset Wad<Price(USD/asset)>.
    let within_band = anchor_ratio_bps(env, anchor, primary)
        .is_some_and(|r| ratio_in_band(r, tolerance.upper_ratio_bps, tolerance.lower_ratio_bps));
    if !within_band {
        panic_with_error!(env, OracleError::UnsafePriceNotAllowed);
    }
    midpoint_price(env, anchor, primary)
}

fn midpoint_price(env: &Env, anchor_price: i128, primary_price: i128) -> i128 {
    anchor_price
        .checked_add(primary_price)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
        / 2
}

/// Primary/anchor ratio in BPS. `None` when the pair is out of band by
/// construction: a zero anchor (undefined ratio) or a ratio that would overflow
/// the fixed-point narrowing.
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
#[path = "../../tests/oracle/tolerance.rs"]
mod tests;
