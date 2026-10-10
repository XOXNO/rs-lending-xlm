//! Solvency, utilization and liquidity checks on the in-memory [`Cache`].
//!
//! Callers run them after interest accrual and before committing state.

use common::constants::{BPS, LIQUIDATION_BUFFER_BPS, MAX_MARKET_VALUE_RAY, RAY};
use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::math::fp_core::{mul_div_ceil, mul_div_floor_saturating};

use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

/// Panics with `UtilizationAboveMax` if utilization exceeds `params.max_utilization`.
///
/// Utilization here is ceiled debt value over floored supply value, rounded up.
/// Skipped when there is no supply or no debt, or when max utilization is
/// effectively unbounded (`>= RAY 1.0`).
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO || cache.params().max_utilization >= Ray::ONE {
        return;
    }

    let borrowed = cache.borrowed().mul_ceil(env, cache.borrow_index());
    if borrowed == Ray::ZERO {
        return;
    }
    let supplied = cache.supplied().mul_floor(env, cache.supply_index());
    assert_with_error!(
        env,
        supplied > Ray::ZERO && borrowed.div_ceil(env, supplied) <= cache.params().max_utilization,
        CollateralError::UtilizationAboveMax
    );
}

/// Panics with `InsufficientLiquidity` if drawing `draw` leaves cash below the liquidation buffer.
///
/// Every debt mint checks it, borrows and strategy openings alike (INV-ACCT-07). Exits do not.
pub(crate) fn require_liquidation_buffer(env: &Env, cache: &Cache, draw: i128) {
    let supplied = cache.unscale_supply_floor(cache.supplied());
    let reserved = mul_div_ceil(env, supplied, LIQUIDATION_BUFFER_BPS, BPS);
    assert_with_error!(
        env,
        cache.cash().saturating_sub(draw) >= reserved,
        CollateralError::InsufficientLiquidity
    );
}

/// Panics with `PoolInsolvent` if the market has a positive [`backing_shortfall`].
pub(crate) fn require_backed_market(env: &Env, cache: &Cache) {
    assert_with_error!(
        env,
        backing_shortfall(cache) == 0,
        CollateralError::PoolInsolvent
    );
}

/// Whole asset units by which supplier claims exceed cash + debt (0 if solvent).
///
/// Claims round down and debt rounds up at RAY precision; the difference
/// floors to asset units once, then cash is subtracted. The result is the
/// floor of the RAY-precision gap, so accrual, which adds the same interest to
/// claims and debt up to RAY-precision rounding, does not turn a gap below one
/// unit into a shortfall.
pub(crate) fn backing_shortfall(cache: &Cache) -> i128 {
    let env = cache.env();
    let claims = cache.supplied().mul_floor(env, cache.supply_index());
    let debt = cache.unscale_borrow_ceil_ray(cache.borrowed());
    if claims <= debt {
        return 0;
    }
    let uncovered = claims
        .checked_sub(env, debt)
        .to_asset_floor(env, cache.params().asset_decimals);
    uncovered.saturating_sub(cache.cash()).max(0)
}

/// Panics with `MathOverflow` if total supply value or total debt value exceeds
/// [`MAX_MARKET_VALUE_RAY`].
///
/// Supply entry and flash and strategy fee booking raise total supply value
/// outside accrual; accrual keeps both totals below the ceiling on its own.
pub(crate) fn require_market_value_within_ceiling(env: &Env, cache: &Cache) {
    let supplied =
        mul_div_floor_saturating(env, cache.supplied().raw(), cache.supply_index().raw(), RAY);
    let borrowed =
        mul_div_floor_saturating(env, cache.borrowed().raw(), cache.borrow_index().raw(), RAY);
    assert_with_error!(
        env,
        supplied <= MAX_MARKET_VALUE_RAY && borrowed <= MAX_MARKET_VALUE_RAY,
        GenericError::MathOverflow
    );
}

/// Panics with `PoolInsolvent` if supplied is zero while borrowed debt is non-zero.
pub(crate) fn require_supply_for_debt(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO && cache.borrowed() != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

#[cfg(test)]
#[path = "../tests/guards.rs"]
mod tests;
