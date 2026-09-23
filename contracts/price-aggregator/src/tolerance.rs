//! Tolerance-band check and midpoint for the two legs of a dual-source price.
//! The band is a ratio in BPS, not a deviation width.

use common::constants::BPS;
use common::math::fp_core;
use common::types::OracleTolerance;
use soroban_sdk::Env;

/// Returns whether the ratio `larger * BPS / smaller` of `anchor` and
/// `primary`, rounded half up, is at most `tolerance.upper_ratio_bps`
/// (`10_500` allows a 5% gap). Returns `false` when the smaller value is not
/// positive or the ratio overflows `i128`.
pub(crate) fn within_tolerance_band(
    env: &Env,
    anchor: i128,
    primary: i128,
    tolerance: &OracleTolerance,
) -> bool {
    let high = anchor.max(primary);
    let low = anchor.min(primary);
    let Some(upper_ratio_bps) = fp_core::try_mul_div_half_up(env, high, BPS, low) else {
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

#[cfg(test)]
#[path = "../tests/oracle/tolerance.rs"]
mod tests;
