//! Market-level solvency and utilization guards. The controller owns account
//! health; these guards protect the pool's own books and are the last check
//! before a mutation is persisted. See `docs/reference/invariants.md`.

use common::errors::CollateralError;
use common::math::fp::Ray;

use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

/// Rejects a mutation that leaves utilization above the market cap.
pub(crate) fn require_utilization_below_max(env: &Env, cache: &Cache) {
    // RAY is the disabled sentinel. Utilization exceeds RAY when
    // `borrowed > supplied`; enabled params are validated below RAY.
    if cache.supplied() == Ray::ZERO || cache.params().max_utilization >= Ray::ONE {
        return;
    }
    // Index-aware: index drift can exceed the cap while scaled totals do not.
    let utilization = cache.calculate_utilization();
    assert_with_error!(
        env,
        utilization <= cache.params().max_utilization,
        CollateralError::UtilizationAboveMax
    );
}

/// Rejects fresh supply while existing claims exceed tracked cash plus
/// outstanding debt. Its real target is the residual deficit left behind when
/// `SUPPLY_INDEX_FLOOR_RAW` truncates a bad-debt write-down: that deficit
/// survives later accrual and rewards, so the check cannot be folded into
/// [`require_solvent_withdraw_state`].
pub(crate) fn require_backed_market(env: &Env, cache: &Cache) {
    // Both roundings favour passing — floor the claim, ceil the backing — so
    // rounding dust never bricks supply. Real insolvency is orders above dust.
    let supplied_claim = cache.unscale_supply_floor(cache.supplied());
    let outstanding_debt = cache.unscale_borrow_ceil(cache.borrowed());
    let backing = cache.cash().saturating_add(outstanding_debt);
    assert_with_error!(
        env,
        supplied_claim <= backing,
        CollateralError::PoolInsolvent
    );
}

/// Rejects a terminal state where debt survives with no supply left to back it.
pub(crate) fn require_solvent_withdraw_state(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO && cache.borrowed() != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

#[cfg(test)]
#[path = "../tests/guards.rs"]
mod tests;
