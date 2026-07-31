use common::rates::{calculate_borrow_rate, calculate_deposit_rate};
use common::types::HubAssetKey;

use soroban_sdk::Env;

use crate::cache::Cache;
use crate::storage;

pub(crate) fn utilization(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    Cache::load(env, hub_asset).calculate_utilization().raw()
}

pub(crate) fn reserves(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    storage::load_state(env, hub_asset).cash
}

pub(crate) fn deposit_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    let util = cache.calculate_utilization();
    let borrow = calculate_borrow_rate(env, util, cache.params());
    calculate_deposit_rate(env, util, borrow, cache.params().reserve_factor).raw()
}

pub(crate) fn borrow_rate(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    calculate_borrow_rate(env, cache.calculate_utilization(), cache.params()).raw()
}

pub(crate) fn protocol_revenue(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply_floor(cache.revenue())
}

pub(crate) fn supplied_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_supply(cache.supplied())
}

pub(crate) fn borrowed_amount(env: &Env, hub_asset: &HubAssetKey) -> i128 {
    let cache = Cache::load(env, hub_asset);
    cache.unscale_borrow(cache.borrowed())
}

pub(crate) fn delta_time(env: &Env, hub_asset: &HubAssetKey) -> u64 {
    let cache = Cache::load(env, hub_asset);
    cache.elapsed_ms()
}

#[cfg(test)]
#[path = "../tests/views.rs"]
mod tests;
