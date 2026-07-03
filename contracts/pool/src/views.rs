use common::errors::GenericError;
use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::{HubAssetKey, MarketParamsRaw, PoolKey, PoolStateRaw, PoolSyncData};
use soroban_sdk::{panic_with_error, Env};

use crate::cache::Cache;

// Raw keyed reads without TTL renewal.
pub fn load_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

pub fn load_params(env: &Env, hub_asset: &HubAssetKey) -> MarketParamsRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::Params(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

// Loads params and state without TTL renewal or interest accrual.
pub fn load_sync_data(env: &Env, hub_asset: &HubAssetKey) -> PoolSyncData {
    PoolSyncData {
        params: load_params(env, hub_asset),
        state: load_state(env, hub_asset),
    }
}

// Capital utilization ratio in RAY from the last persisted checkpoint.
// No interest accrual.
pub fn capital_utilisation(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    Cache::load(env, hub_asset).calculate_utilization().raw()
}

// Returns persisted `cash`; direct token donations are excluded.
pub fn reserves(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    load_state(env, hub_asset).cash
}

// Current deposit APR in RAY. Does not trigger interest accrual.
pub fn deposit_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, &cache.params);
    calculate_deposit_rate(env, util, borrow, cache.params.reserve_factor).raw()
}

// Current borrow APR in RAY. Does not trigger interest accrual.
pub fn borrow_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    calculate_borrow_rate(env, cache.calculate_utilization(), &cache.params).raw()
}

// Accrued protocol revenue in asset decimals. Does not trigger interest accrual.
pub fn protocol_revenue(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.revenue)
}

// Total supplied in asset decimals. Does not trigger interest accrual.
pub fn supplied_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.supplied)
}

// Total borrowed in asset decimals. Does not trigger interest accrual.
pub fn borrowed_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_borrow(cache.borrowed)
}

// Milliseconds elapsed since last accrual. Does not trigger interest accrual.
pub fn delta_time(env: &Env, hub_asset: &HubAssetKey) -> u64 {
    let cache = Cache::load(env, hub_asset);

    cache.current_timestamp.saturating_sub(cache.last_timestamp)
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;
