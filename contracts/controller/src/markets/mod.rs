//! Pool lifecycle: the one-time pool deployment, market creation, rate-model
//! replacement, and pool WASM upgrade. Owner-gated only, so every entry rides
//! the governance timelock.

use common::errors::GenericError;
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw};
use soroban_sdk::{assert_with_error, Address, BytesN, Env};

use crate::config;
use crate::context::Cache;
use crate::events::{CreateMarketEvent, UpdateMarketParamsEvent};
use crate::external::pool::{
    pool_create_market_call, pool_update_indexes_call, pool_update_params_call, pool_upgrade_call,
};
use crate::storage;

/// Deterministic salt for the one-time central pool deployment; the pool
/// address derives from (controller address, salt).
const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];

pub(crate) fn deploy_pool(env: &Env, wasm_hash: BytesN<32>) -> Address {
    storage::renew_controller_instance(env);

    assert_with_error!(
        env,
        storage::try_get_pool(env).is_none(),
        GenericError::PoolAlreadyDeployed
    );

    let salt = BytesN::from_array(env, &POOL_DEPLOY_SALT);
    let pool = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash, (env.current_contract_address(),));

    storage::set_pool(env, &pool);
    pool
}

pub(crate) fn create_liquidity_pool(
    env: &Env,
    hub_id: u32,
    asset: Address,
    params: MarketParamsRaw,
) -> Address {
    config::require_hub_active(env, hub_id);

    assert_with_error!(env, params.asset_id == asset, GenericError::WrongToken);

    let pool_address = storage::get_pool(env);
    pool_create_market_call(env, &pool_address, hub_id, &params);

    storage::renew_controller_instance(env);

    CreateMarketEvent {
        hub_id,
        base_asset: asset.clone(),
        max_borrow_rate: params.max_borrow_rate,
        base_borrow_rate: params.base_borrow_rate,
        slope1: params.slope1,
        slope2: params.slope2,
        slope3: params.slope3,
        mid_utilization: params.mid_utilization,
        optimal_utilization: params.optimal_utilization,
        max_utilization: params.max_utilization,
        reserve_factor: params.reserve_factor,
        market_address: pool_address.clone(),
    }
    .publish(env);

    pool_address
}

pub(crate) fn upgrade_liquidity_pool_params(
    env: &Env,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    let mut cache = Cache::new(env);

    let pool_addr = cache.cached_pool_address();

    // `update_indexes` reverts `PoolNotInitialized` for an uncreated market.
    pool_update_indexes_call(env, &pool_addr, hub_asset);

    pool_update_params_call(env, &pool_addr, hub_asset, params);

    UpdateMarketParamsEvent {
        asset: hub_asset.asset.clone(),
        max_borrow_rate: params.max_borrow_rate,
        base_borrow_rate: params.base_borrow_rate,
        slope1: params.slope1,
        slope2: params.slope2,
        slope3: params.slope3,
        mid_utilization: params.mid_utilization,
        optimal_utilization: params.optimal_utilization,
        max_utilization: params.max_utilization,
        reserve_factor: params.reserve_factor,
    }
    .publish(env);
}

pub(crate) fn upgrade_pool(env: &Env, new_wasm_hash: BytesN<32>) {
    storage::renew_controller_instance(env);
    let pool_addr = storage::get_pool(env);
    pool_upgrade_call(env, &pool_addr, &new_wasm_hash);
}
