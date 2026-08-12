//! Helpers for comparing an anchor price against a primary price within a
//! configured tolerance band, expressed in basis points.

use common::constants::BPS;
use common::math::fp_core;
use common::types::OracleTolerance;
use soroban_sdk::Env;

/// Returns whether `anchor` and `primary` are within `tolerance`'s
/// `upper_ratio_bps` of each other, computed as the ratio in basis points
/// between the larger and the smaller of the two values. Returns `false`
/// if that ratio cannot be computed, for example because the smaller value
/// is non-positive or the computation overflows.
pub(crate) fn within_tolerance_band(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OracleTolerance,
) -> bool {
    let high = anchor.max(primary);
    let low = anchor.min(primary);
    let Some(upper_ratio_bps) = ratio_bps(env, high, low) else {
        return false;
    };

    upper_ratio_bps <= i128::from(tolerance.upper_ratio_bps)
}

/// Returns the average of `anchor_price` and `primary_price`, truncated
/// toward zero. Returns `0` if the sum overflows `i128`.
pub(crate) fn midpoint_price_or_zero(anchor_price: i128, primary_price: i128) -> i128 {
    anchor_price
        .checked_add(primary_price)
        .map(|sum| sum / 2)
        .unwrap_or(0)
}

/// Computes `numerator / denominator` expressed in basis points, rounded
/// half up. Returns `None` if `numerator` is negative, `denominator` is
/// non-positive, or the result does not fit in `i128`.
fn ratio_bps(env: &Env, numerator: i128, denominator: i128) -> Option<i128> {
    fp_core::try_mul_div_half_up(env, numerator, BPS, denominator)
}

#[cfg(test)]
#[path = "../tests/oracle/tolerance.rs"]
mod tests;
