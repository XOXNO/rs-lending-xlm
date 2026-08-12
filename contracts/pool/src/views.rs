//! Read-only market queries used by public view entrypoints.
//!
//! Views load a [`Cache`] (or raw state for cash) without committing accrual.
//! Rates and utilization therefore reflect **stored** indexes unless a prior
//! mutation already wrote an update. Rate getters return **annual** RAY APR,
//! not the per-millisecond rate used by accrual.

use common::rates::{calculate_annual_borrow_rate, calculate_deposit_rate};
use common::types::HubAssetKey;

use soroban_sdk::Env;

use crate::cache::Cache;
use crate::storage;

/// Utilization ratio as a raw RAY integer (borrowed value / supplied value).
pub(crate) fn utilization(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    Cache::load(env, hub_asset).calculate_utilization().raw()
}

/// Available cash reserves in asset units.
pub(crate) fn reserves(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    storage::load_state(env, hub_asset).cash
}

/// Supplier APR (annual RAY) at current stored utilization and reserve factor.
pub(crate) fn deposit_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_annual_borrow_rate(env, util, cache.params());
    calculate_deposit_rate(env, util, borrow, cache.params().reserve_factor).raw()
}

/// Borrow APR (annual RAY) from the piecewise interest model at current util.
pub(crate) fn borrow_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    calculate_annual_borrow_rate(env, cache.calculate_utilization(), cache.params()).raw()
}

/// Protocol revenue in asset units (floored conversion of scaled revenue shares).
pub(crate) fn protocol_revenue(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply_floor(cache.revenue())
}

/// Total supplied underlying in asset units.
pub(crate) fn supplied_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.supplied())
}

/// Total borrowed underlying in asset units.
pub(crate) fn borrowed_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_borrow(cache.borrowed())
}

/// Milliseconds since last accrual (`current_timestamp - last_timestamp`).
pub(crate) fn delta_time(env: &Env, hub_asset: &HubAssetKey) -> u64 {
    let cache = Cache::load(env, hub_asset);
    cache.elapsed_ms()
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;
