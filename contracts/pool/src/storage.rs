//! Persistent storage access for market params, market state, and TTL renewal.
//!
//! Markets are keyed by [`HubAssetKey`] via [`PoolKey::Params`] and
//! [`PoolKey::State`]. All reads of a missing market panic with
//! [`GenericError::PoolNotInitialized`].

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;
use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PoolKey, PoolStateRaw, PoolSyncData,
};

use soroban_sdk::{panic_with_error, Env};

/// Returns `true` if market params for `hub_asset` exist in persistent storage.
pub(crate) fn market_exists(env: &Env, hub_asset: &HubAssetKey) -> bool {
    env.storage()
        .persistent()
        .has(&PoolKey::Params(hub_asset.clone()))
}

/// Loads market parameters, or panics if the market was never created.
pub(crate) fn read_params(env: &Env, hub_asset: &HubAssetKey) -> MarketParamsRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::Params(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Loads market state, or panics if the market was never created.
///
/// Does not extend the TTL; [`load_state`] does.
pub(crate) fn read_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Loads market state and extends the TTL of both params and state keys.
pub(crate) fn load_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    let state = read_state(env, hub_asset);
    renew_market(env, hub_asset);
    state
}

/// Loads params + state as [`PoolSyncData`] and renews market TTLs.
pub(crate) fn load_sync_data(env: &Env, hub_asset: &HubAssetKey) -> PoolSyncData {
    let params = read_params(env, hub_asset);
    let state = read_state(env, hub_asset);
    renew_market(env, hub_asset);
    PoolSyncData { params, state }
}

/// Persists market parameters under `PoolKey::Params`.
pub(crate) fn write_params(env: &Env, hub_asset: &HubAssetKey, params: &MarketParamsRaw) {
    env.storage()
        .persistent()
        .set(&PoolKey::Params(hub_asset.clone()), params);
}

/// Persists market state under `PoolKey::State`.
pub(crate) fn write_state(env: &Env, hub_asset: &HubAssetKey, state: &PoolStateRaw) {
    env.storage()
        .persistent()
        .set(&PoolKey::State(hub_asset.clone()), state);
}

/// Copies every `model` field into the stored params and returns the result.
///
/// Leaves `asset_id` and `asset_decimals` unchanged.
pub(crate) fn write_rate_model(
    env: &Env,
    hub_asset: &HubAssetKey,
    model: &InterestRateModel,
) -> MarketParamsRaw {
    let mut params = read_params(env, hub_asset);

    params.max_borrow_rate = model.max_borrow_rate;
    params.base_borrow_rate = model.base_borrow_rate;
    params.slope1 = model.slope1;
    params.slope2 = model.slope2;
    params.slope3 = model.slope3;
    params.mid_utilization = model.mid_utilization;
    params.optimal_utilization = model.optimal_utilization;
    params.max_utilization = model.max_utilization;
    params.reserve_factor = model.reserve_factor;
    params.is_flashloanable = model.is_flashloanable;
    params.flashloan_fee = model.flashloan_fee;

    write_params(env, hub_asset, &params);
    params
}

/// Extends the persistent TTL of a market's params and state.
pub(crate) fn renew_market(env: &Env, hub_asset: &HubAssetKey) {
    let storage = env.storage().persistent();
    storage.extend_ttl(
        &PoolKey::Params(hub_asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
    storage.extend_ttl(
        &PoolKey::State(hub_asset.clone()),
        TTL_THRESHOLD_SHARED,
        TTL_BUMP_SHARED,
    );
}
