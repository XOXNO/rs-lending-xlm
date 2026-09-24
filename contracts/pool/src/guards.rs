//! Solvency, utilization and liquidity checks on the in-memory [`Cache`].
//!
//! Callers run them after interest accrual and before committing state.

use common::errors::CollateralError;
use common::math::fp::Ray;

use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

/// Panics with `UtilizationAboveMax` if utilization exceeds `params.max_utilization`.
///
/// Skipped when there is no supply, or when max utilization is effectively
/// unbounded (`>= RAY 1.0`).
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO || cache.params().max_utilization >= Ray::ONE {
        return;
    }

    let utilization = cache.calculate_utilization();
    assert_with_error!(
        env,
        utilization <= cache.params().max_utilization,
        CollateralError::UtilizationAboveMax
    );
}

/// Panics with `InsufficientLiquidity` if drawing `draw` leaves cash below the liquidation buffer.
///
/// Every debt mint checks it, borrows and strategy openings alike (INV-ACCT-07). Exits do not.
pub(crate) fn require_liquidation_buffer(env: &Env, cache: &Cache, draw: i128) {
    let supplied = cache.unscale_supply_floor(cache.supplied());
    let reserved = common::math::fp::Bps::from(common::constants::LIQUIDATION_BUFFER_BPS)
        .apply_to(env, supplied);
    assert_with_error!(
        env,
        cache.cash().saturating_sub(draw) >= reserved,
        CollateralError::InsufficientLiquidity
    );
}

/// Panics with `PoolInsolvent` if the market has a positive backing shortfall.
///
/// Backing = cash + ceiled debt value; claims = floored supply value.
pub(crate) fn require_backed_market(env: &Env, cache: &Cache) {
    assert_with_error!(
        env,
        backing_shortfall(cache) == 0,
        CollateralError::PoolInsolvent
    );
}

/// Asset units by which supplier claims exceed cash + debt (0 if solvent).
pub(crate) fn backing_shortfall(cache: &Cache) -> i128 {
    let supplied_claim = cache.unscale_supply_floor(cache.supplied());
    let outstanding_debt = cache.unscale_borrow_ceil(cache.borrowed());
    let backing = cache.cash().saturating_add(outstanding_debt);
    supplied_claim.saturating_sub(backing).max(0)
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
