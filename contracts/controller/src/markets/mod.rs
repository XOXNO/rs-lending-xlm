//! Deploys and upgrades the liquidity-pool contract, and creates or updates
//! markets on it through cross-contract calls.

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

/// Fixed salt used when deploying the liquidity-pool contract instance.
const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];

/// Deploys the liquidity-pool contract under the controller's own address using a
/// fixed salt, and stores its address. Panics if a pool is already deployed.
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

/// Registers a new market for `asset` under `hub_id` on the deployed liquidity pool
/// and publishes a `CreateMarketEvent`. Panics if the hub is not active or if
/// `params.asset_id` does not match `asset`.
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

    CreateMarketEvent::from_params(hub_id, asset, pool_address.clone(), &params).publish(env);

    pool_address
}

/// Updates the interest-rate model for the market identified by `hub_asset` on the
/// pool, first accruing its indexes up to date, and publishes an
/// `UpdateMarketParamsEvent` with the new parameters.
pub(crate) fn upgrade_liquidity_pool_params(
    env: &Env,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    let mut cache = Cache::new(env);

    let pool_addr = cache.cached_pool_address();

    pool_update_indexes_call(env, &pool_addr, hub_asset);

    pool_update_params_call(env, &pool_addr, hub_asset, params);

    UpdateMarketParamsEvent::from((hub_asset.hub_id, hub_asset.asset.clone(), params)).publish(env);
}

/// Upgrades the deployed liquidity-pool contract to the WASM at `new_wasm_hash`.
pub(crate) fn upgrade_pool(env: &Env, new_wasm_hash: BytesN<32>) {
    storage::renew_controller_instance(env);
    let pool_addr = storage::get_pool(env);
    pool_upgrade_call(env, &pool_addr, &new_wasm_hash);
}
