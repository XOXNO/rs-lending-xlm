use common::errors::CollateralError;
use common::math::fp::Ray;

use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;

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

pub(crate) fn require_backed_market(env: &Env, cache: &Cache) {
    assert_with_error!(
        env,
        backing_shortfall(cache) == 0,
        CollateralError::PoolInsolvent
    );
}

pub(crate) fn backing_shortfall(cache: &Cache) -> i128 {
    let supplied_claim = cache.unscale_supply_floor(cache.supplied());
    let outstanding_debt = cache.unscale_borrow_ceil(cache.borrowed());
    let backing = cache.cash().saturating_add(outstanding_debt);
    supplied_claim.saturating_sub(backing).max(0)
}

pub(crate) fn require_solvent_withdraw_state(env: &Env, cache: &Cache) {
    if cache.supplied() == Ray::ZERO && cache.borrowed() != Ray::ZERO {
        panic_with_error!(env, CollateralError::PoolInsolvent);
    }
}

#[cfg(test)]
#[path = "../tests/guards.rs"]
mod tests;
