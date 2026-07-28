//! Checkpoint market reads for the view ABI. Every value here reflects the last
//! persisted accrual — nothing accrues or writes. Live indexes come from
//! `get_bulk_indexes`.

use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::HubAssetKey;

use soroban_sdk::Env;

use crate::cache::Cache;
use crate::storage;

/// Checkpoint utilization in RAY; zero on an empty market.
pub(crate) fn utilization(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    Cache::load(env, hub_asset).calculate_utilization().raw()
}

/// Returns tracked `cash`; direct token donations are excluded.
pub(crate) fn reserves(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    storage::load_state(env, hub_asset).cash
}

/// Checkpoint deposit rate in RAY.
pub(crate) fn deposit_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, cache.params());
    calculate_deposit_rate(env, util, borrow, cache.params().reserve_factor).raw()
}

/// Checkpoint borrow rate in RAY.
pub(crate) fn borrow_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    calculate_borrow_rate(env, cache.calculate_utilization(), cache.params()).raw()
}

/// Returns the floored protocol revenue claim. `claim_revenue`'s actual payout
/// is this value capped by tracked cash (`cash.min(claim)`), so the two
/// diverge whenever the market is cash-short.
pub(crate) fn protocol_revenue(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply_floor(cache.revenue())
}

/// Half-up total supplied value: a reporting figure, not a payable amount.
pub(crate) fn supplied_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.supplied())
}

/// Half-up total borrowed value: a reporting figure, not the debt owed.
pub(crate) fn borrowed_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_borrow(cache.borrowed())
}

/// Returns milliseconds since the market last accrued interest.
pub(crate) fn delta_time(env: &Env, hub_asset: &HubAssetKey) -> u64 {
    let cache = Cache::load(env, hub_asset);
    cache.elapsed_ms()
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;
