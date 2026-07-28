//! Dual-source agreement band: pure ratio math shared by hard and soft paths.
//!
//! The engine sets `Outcome.deviation` from [`within_tolerance_band`] and blends
//! dual legs with [`midpoint_price_or_zero`]. Disagreement maps through
//! [`crate::engine::force`] to `UnsafePriceNotAllowed`.

use common::constants::{BPS_DECIMALS, RAY, RAY_DECIMALS};
use common::math::fp_core;
use common::types::OracleTolerance;
use soroban_sdk::Env;

/// True when `primary / anchor` lies in the inclusive BPS band on `tolerance`.
/// Symmetric in the sense that the band is applied to that single ratio.
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

/// Integer midpoint of the two prices, or `0` on add overflow.
///
/// Hard path maps `0` to `InvalidPrice`; soft path marks invalid.
pub(crate) fn midpoint_price_or_zero(anchor_price: i128, primary_price: i128) -> i128 {
    anchor_price
        .checked_add(primary_price)
        .map(|sum| sum / 2)
        .unwrap_or(0)
}

/// `primary / anchor` in BPS. `None` when the ratio is undefined (zero anchor)
/// or would overflow the fixed-point narrowing.
fn anchor_ratio_bps(env: &Env, anchor: i128, primary: i128) -> Option<i128> {
    if anchor == 0 {
        return None;
    }
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

fn ratio_in_band(ratio_bps: i128, upper_bound_ratio: u32, lower_bound_ratio: u32) -> bool {
    ratio_bps <= i128::from(upper_bound_ratio) && ratio_bps >= i128::from(lower_bound_ratio)
}

#[cfg(test)]
#[path = "../tests/oracle/tolerance.rs"]
mod tests;
