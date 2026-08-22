use common::errors::GenericError;
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw};
use soroban_sdk::{assert_with_error, Address, BytesN, Env, String};

use crate::config;
use crate::context::Cache;
use crate::events::{CreateMarketEvent, UpdateMarketParamsEvent};
use crate::external::pool::{
    pool_create_market_call, pool_update_indexes_call, pool_update_params_call, pool_upgrade_call,
};
use crate::external::position_nft::nft_upgrade_call;
use crate::storage;
use swap_aggregator_interface::SwapAggregatorClient;

const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];
const POSITION_NFT_DEPLOY_SALT: [u8; 32] = [1u8; 32];

/// Deploys the pool contract from `wasm_hash` at a fixed deployment salt,
/// records its address, and returns it. Panics if a pool has already been
/// deployed.
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

/// Deploys the position-NFT contract from `wasm_hash` at a fixed salt distinct
/// from the pool's, passing the controller itself as the NFT's authorized
/// minter/burner, records the address, and returns it. Panics if already
/// deployed.
pub(crate) fn deploy_position_nft(
    env: &Env,
    wasm_hash: BytesN<32>,
    uri: String,
    name: String,
    symbol: String,
) -> Address {
    storage::renew_controller_instance(env);

    assert_with_error!(
        env,
        storage::try_get_position_nft(env).is_none(),
        GenericError::PositionNftAlreadyDeployed
    );

    let salt = BytesN::from_array(env, &POSITION_NFT_DEPLOY_SALT);
    let nft = env.deployer().with_current_contract(salt).deploy_v2(
        wasm_hash,
        (env.current_contract_address(), uri, name, symbol),
    );

    storage::set_position_nft(env, &nft);
    nft
}

/// Creates a new market for `asset` under hub `hub_id` on the pool contract
/// and publishes a `CreateMarketEvent`, returning the pool's address.
/// Panics if the hub is not active or if `params.asset_id` does not match
/// `asset`.
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

/// Accrues `hub_asset`'s indexes on the pool under the current rate model,
/// then applies the new interest-rate model `params` and publishes an
/// `UpdateMarketParamsEvent`.
pub(crate) fn upgrade_liquidity_pool_params(
    env: &Env,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    let mut cache = Cache::new(env);

    let pool_addr = cache.cached_pool_address();

    pool_update_indexes_call(env, &pool_addr, hub_asset);

    pool_update_params_call(env, &pool_addr, hub_asset, params);

    UpdateMarketParamsEvent::from_rate_model(hub_asset.hub_id, hub_asset.asset.clone(), params)
        .publish(env);
}

/// Renews the controller's storage TTL and upgrades the pool contract's
/// Wasm bytecode to `new_wasm_hash`.
pub(crate) fn upgrade_pool(env: &Env, new_wasm_hash: BytesN<32>) {
    storage::renew_controller_instance(env);
    let pool_addr = storage::get_pool(env);
    pool_upgrade_call(env, &pool_addr, &new_wasm_hash);
}

/// Renews the controller's storage TTL and upgrades the position-NFT
/// contract's Wasm bytecode to `new_wasm_hash`. Panics with
/// `PositionNftNotSet` when the NFT has not been deployed.
pub(crate) fn upgrade_position_nft(env: &Env, new_wasm_hash: BytesN<32>) {
    storage::renew_controller_instance(env);
    let nft_addr = storage::get_position_nft(env);
    nft_upgrade_call(env, &nft_addr, &new_wasm_hash);
}

/// Renews the controller TTL and upgrades the swap-aggregator router. Requires
/// the controller to be the router's owner (set at deploy). Reached only from
/// the owner-gated `upgrade_swap_aggregator` entrypoint (B-2).
pub(crate) fn upgrade_swap_aggregator(env: &Env, new_wasm_hash: BytesN<32>) {
    storage::renew_controller_instance(env);
    let router = storage::get_swap_aggregator(env);
    SwapAggregatorClient::new(env, &router).upgrade(&new_wasm_hash);
}
