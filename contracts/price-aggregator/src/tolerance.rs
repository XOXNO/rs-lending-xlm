//! Dual-source agreement band: pure ratio math shared by hard and soft paths.
//!
//! The engine sets `Outcome.deviation` from [`within_tolerance_band`] and blends
//! dual legs with [`midpoint_price_or_zero`]. Disagreement maps through
//! [`crate::engine::force`] to `UnsafePriceNotAllowed`.

use common::constants::BPS;
use common::math::fp_core;
use common::types::OracleTolerance;
use soroban_sdk::Env;

/// True when the canonical high/low pair lies in the inclusive reciprocal BPS
/// band on `tolerance`. Canonicalization makes source order unobservable.
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
    // The validated lower bound is the rounded reciprocal of `upper`. Applying
    // both rounded directions would reintroduce source-order asymmetry, so the
    // canonical high/low ratio is the single runtime decision.
    upper_ratio_bps <= i128::from(tolerance.upper_ratio_bps)
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

/// `numerator / denominator` in BPS with one I256 half-up operation.
fn ratio_bps(env: &Env, numerator: i128, denominator: i128) -> Option<i128> {
    fp_core::try_mul_div_half_up(env, numerator, BPS, denominator)
}

#[cfg(test)]
#[path = "../tests/oracle/tolerance.rs"]
mod tests;
