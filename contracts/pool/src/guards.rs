//! Solvency, utilization and liquidity checks on the in-memory [`Cache`].
//!
//! Callers run them after interest accrual and before committing state.

use common::constants::{LIQUIDATION_BUFFER_BPS, MAX_MARKET_ENTRY_RAY};
use common::errors::CollateralError;
use common::math::fp::{Bps, Ray};

use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

/// Panics with `UtilizationAboveMax` if utilization exceeds `params.max_utilization`.
///
/// Debt-creating legs call it (INV-ACCT-08). It compares the ceiled debt value
/// with the floored supply value times the ceiling, rounded down, so rounding
/// cannot admit utilization above the ceiling and no quotient can overflow.
/// A 100% ceiling still binds: debt value may not exceed supply value.
/// Skipped when there are no supply shares or no debt value.
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO {
        return;
    }

    let borrowed = cache.borrowed().mul_ceil(env, cache.borrow_index());
    if borrowed == Ray::ZERO {
        return;
    }
    let supplied = cache.supplied().mul_floor(env, cache.supply_index());
    let allowed = supplied.mul_floor(env, cache.params().max_utilization);
    assert_with_error!(
        env,
        borrowed <= allowed,
        CollateralError::UtilizationAboveMax
    );
}

/// Exit form of [`require_utilization_below_max`], for ordinary withdrawals
/// and revenue claims.
///
/// Skipped at a 100% ceiling. An exit cannot draw more than cash, so there
/// the comparison could only fail on the rounding dust between cash plus
/// debt and supplier claims, and would stop the withdrawal of the last cash.
pub(crate) fn require_exit_utilization_below_max(env: &Env, cache: &Cache) {
    if cache.params().max_utilization >= Ray::ONE {
        return;
    }
    require_utilization_below_max(env, cache);
}

/// Panics with `MarketCeilingReached` if total supply or total debt, as a
/// share count or as a value, exceeds `MAX_MARKET_ENTRY_RAY` (INV-ACCT-11).
///
/// Supply, borrow, strategy and flash-loan entries call it after they raise a
/// total. Exits do not: accrual holds interest at `MAX_MARKET_VALUE_RAY`, so a
/// market above the entry ceiling still accrues, repays, withdraws and
/// liquidates.
pub(crate) fn require_within_entry_ceiling(env: &Env, cache: &Cache) {
    assert_with_error!(
        env,
        fits_entry_ceiling(env, cache.supplied(), cache.supply_index())
            && fits_entry_ceiling(env, cache.borrowed(), cache.borrow_index()),
        CollateralError::MarketCeilingReached
    );
}

/// Returns `true` if `scaled` shares and their half-up value at `index` are
/// both at most `MAX_MARKET_ENTRY_RAY`.
fn fits_entry_ceiling(env: &Env, scaled: Ray, index: Ray) -> bool {
    let ceiling = Ray::from(MAX_MARKET_ENTRY_RAY);
    if scaled > ceiling {
        return false;
    }
    match scaled.try_mul(env, index) {
        Some(value) => value <= ceiling,
        None => false,
    }
}

/// Panics with `InsufficientLiquidity` if drawing `draw` leaves cash below the liquidation buffer.
///
/// Every debt mint checks it, borrows and strategy openings alike (INV-ACCT-07). Exits do not.
pub(crate) fn require_liquidation_buffer(env: &Env, cache: &Cache, draw: i128) {
    let supplied = cache.unscale_supply_floor(cache.supplied());
    let reserved = Bps::from(LIQUIDATION_BUFFER_BPS).apply_to_ceil(env, supplied);
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
///
/// Floored claims minus ceiled debt, at RAY precision, floor to asset units
/// once before cash is subtracted, so the shortfall is never over-reported.
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

/// Panics with `PoolInsolvent` if supplied is zero while borrowed debt is non-zero.
pub(crate) fn require_supply_for_debt(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO && cache.borrowed() != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

#[cfg(test)]
#[path = "../tests/guards.rs"]
mod tests;
