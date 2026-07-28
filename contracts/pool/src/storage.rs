//! Sole owner of `PoolKey`. Every persistent market read, write, and TTL
//! renewal in this contract goes through this module, so key shape and TTL
//! policy have exactly one definition.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::GenericError;
use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PoolKey, PoolStateRaw, PoolSyncData,
};

use soroban_sdk::{panic_with_error, Env};

/// True once `create_market` has stored params for this market.
pub(crate) fn market_exists(env: &Env, hub_asset: &HubAssetKey) -> bool {
    env.storage()
        .persistent()
        .has(&PoolKey::Params(hub_asset.clone()))
}

/// Reads market params without renewing TTL.
pub(crate) fn read_params(env: &Env, hub_asset: &HubAssetKey) -> MarketParamsRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::Params(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Reads market accounting state without renewing TTL.
pub(crate) fn read_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub_asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Reads market accounting state and renews both market keys.
pub(crate) fn load_state(env: &Env, hub_asset: &HubAssetKey) -> PoolStateRaw {
    let state = read_state(env, hub_asset);
    renew_market(env, hub_asset);
    state
}

/// Reads params and state together and renews both market keys once.
pub(crate) fn load_sync_data(env: &Env, hub_asset: &HubAssetKey) -> PoolSyncData {
    let params = read_params(env, hub_asset);
    let state = read_state(env, hub_asset);
    renew_market(env, hub_asset);
    PoolSyncData { params, state }
}

/// Writes params without renewing TTL; pair with [`renew_market`].
pub(crate) fn write_params(env: &Env, hub_asset: &HubAssetKey, params: &MarketParamsRaw) {
    env.storage()
        .persistent()
        .set(&PoolKey::Params(hub_asset.clone()), params);
}

/// Writes accounting state without renewing TTL; pair with [`renew_market`].
pub(crate) fn write_state(env: &Env, hub_asset: &HubAssetKey, state: &PoolStateRaw) {
    env.storage()
        .persistent()
        .set(&PoolKey::State(hub_asset.clone()), state);
}

/// Overwrites the market's interest-rate parameters and returns the stored row.
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

/// Renews the contract instance entry.
pub(crate) fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Renews both market keys. Both must already exist — `extend_ttl` traps on a
/// missing entry, so only call this after a successful read or write.
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
