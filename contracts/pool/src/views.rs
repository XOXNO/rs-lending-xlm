//! Checkpoint market reads for the view ABI. No accrual; live indexes via
//! `get_bulk_indexes`. See `architecture/INVARIANTS.md`.

use common::errors::GenericError;
use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::{HubAssetKey, MarketParamsRaw, PoolKey, PoolStateRaw, PoolSyncData};

use soroban_sdk::{panic_with_error, Env};

use crate::cache::Cache;
use crate::utils;

pub fn load_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    let key = PoolKey::State(hub_asset.clone());
    let v = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
    utils::renew_market_keys(env, hub_asset);
    v
}

pub fn load_params(env: &Env, hub_asset: &HubAssetKey) -> MarketParamsRaw {
    let key = PoolKey::Params(hub_asset.clone());
    let v = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized));
    utils::renew_market_keys(env, hub_asset);
    v
}

pub fn load_sync_data(env: &Env, hub_asset: &HubAssetKey) -> PoolSyncData {
    PoolSyncData {
        params: load_params(env, hub_asset),
        state: load_state(env, hub_asset),
    }
}

pub fn capital_utilisation(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    Cache::load(env, hub_asset).calculate_utilization().raw()
}

/// Persisted `cash`; direct token donations excluded.
pub fn reserves(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    load_state(env, hub_asset).cash
}

pub fn deposit_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, &cache.params);
    calculate_deposit_rate(env, util, borrow, cache.params.reserve_factor).raw()
}

pub fn borrow_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    calculate_borrow_rate(env, cache.calculate_utilization(), &cache.params).raw()
}

/// Floored to match what `claim_revenue` actually pays out.
pub fn protocol_revenue(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply_floor(cache.revenue)
}

pub fn supplied_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.supplied)
}

pub fn borrowed_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_borrow(cache.borrowed)
}

pub fn delta_time(env: &Env, hub_asset: &HubAssetKey) -> u64 {
    let cache = Cache::load(env, hub_asset);
    cache.current_timestamp.saturating_sub(cache.last_timestamp)
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;
