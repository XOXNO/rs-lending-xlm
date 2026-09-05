use common::errors::{GenericError, OracleError};
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw};
use common::validation::require_positive_amount;
use soroban_sdk::{
    assert_with_error, panic_with_error, token, vec, Address, BytesN, Env, String, Vec,
};

use crate::config;
use crate::context::Context;
use crate::events::{CreateMarketEvent, UpdateMarketParamsEvent};
use crate::external::pool::{
    pool_claim_revenue_call, pool_create_market_call, pool_recapitalize_call,
    pool_update_indexes_call, pool_update_params_call, pool_upgrade_call,
};
use crate::external::position_nft::nft_upgrade_call;
use crate::payments::balance_delta_since;
use crate::risk::validation;
use crate::{events, payments, storage};

const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];
const POSITION_NFT_DEPLOY_SALT: [u8; 32] = [1u8; 32];

/// Deploys and records the sole pool at a fixed salt; rejects a second deployment.
pub(crate) fn deploy_pool(env: &Env, wasm_hash: BytesN<32>) -> Address {
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

/// Deploys and records the position NFT with the controller as minter/burner.
/// Uses a salt distinct from the pool and rejects a second deployment.
pub(crate) fn deploy_position_nft(
    env: &Env,
    wasm_hash: BytesN<32>,
    uri: String,
    name: String,
    symbol: String,
) -> Address {
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

/// Creates a pool market in an active hub and emits its configuration.
/// Requires `params.asset_id` to match `asset`; returns the pool address.
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

    CreateMarketEvent::from_params(hub_id, asset, pool_address.clone(), &params).publish(env);

    pool_address
}

/// Accrues indexes under the current model before replacing rate and flash-loan
/// parameters, then emits the new configuration.
pub(crate) fn upgrade_liquidity_pool_params(
    env: &Env,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    let mut cache = Context::new(env);

    let pool_addr = cache.cached_pool_address();

    pool_update_indexes_call(env, &pool_addr, &vec![env, hub_asset.clone()]);

    pool_update_params_call(env, &pool_addr, hub_asset, params);

    UpdateMarketParamsEvent::from_rate_model(hub_asset.hub_id, hub_asset.asset.clone(), params)
        .publish(env);
}

/// Upgrades the pool Wasm to `new_wasm_hash`.
pub(crate) fn upgrade_pool(env: &Env, new_wasm_hash: BytesN<32>) {
    let pool_addr = storage::get_pool(env);
    pool_upgrade_call(env, &pool_addr, &new_wasm_hash);
}

/// Upgrades the position NFT Wasm; fails if the NFT is unconfigured.
pub(crate) fn upgrade_position_nft(env: &Env, new_wasm_hash: BytesN<32>) {
    let nft_addr = storage::get_position_nft(env);
    nft_upgrade_call(env, &nft_addr, &new_wasm_hash);
}

/// Accrues indexes for each hub asset. Requires caller authorization and no flash loan.
pub(crate) fn update_indexes(env: &Env, caller: Address, assets: Vec<HubAssetKey>) {
    validation::require_authorized_caller(env, &caller);

    let mut cache = Context::new(env);
    let pool_addr = cache.cached_pool_address();
    pool_update_indexes_call(env, &pool_addr, &assets);
}

/// Claims and forwards revenue to the accumulator in input order. Returns
/// measured controller receipts; requires caller authorization and no flash loan.
pub(crate) fn claim_revenue(env: &Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
    validation::require_authorized_caller(env, &caller);
    let mut results = Vec::new(env);
    let mut cache = Context::new(env);
    for hub_asset in assets {
        let amount = claim_revenue_for_asset(env, &caller, &hub_asset, &mut cache);
        results.push_back(amount);
    }
    results
}

/// Transfers funds to the pool, credits the measured receipt up to the backing
/// shortfall, and refunds unused funds. Returns credited cash; rejects flash loans.
pub(crate) fn recapitalize(
    env: &Env,
    payer: Address,
    hub_asset: HubAssetKey,
    amount: i128,
) -> i128 {
    validation::require_authorized_caller(env, &payer);
    require_positive_amount(env, amount);

    let mut cache = Context::new(env);
    let pool_addr = cache.cached_pool_address();
    // Prefund the pool and credit only its measured receipt.
    let received = payments::transfer_amount_measured(
        env,
        &hub_asset.asset,
        &payer,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    pool_recapitalize_call(env, &pool_addr, &hub_asset, &payer, received).actual_amount
}

/// Claims one market's revenue and forwards the measured controller receipt.
/// Requires an accumulator; emits a revenue event only for positive receipts.
fn claim_revenue_for_asset(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    cache: &mut Context,
) -> i128 {
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));

    let pool_addr = cache.cached_pool_address();

    // Measure custody receipts before forwarding inexact-delivery tokens (INV-ACCT-03).
    let controller = env.current_contract_address();
    let asset = &hub_asset.asset;
    let before = token::Client::new(env, asset).balance(&controller);

    let _ = pool_claim_revenue_call(env, &pool_addr, hub_asset);

    let received = balance_delta_since(env, asset, &controller, before);

    if received > 0 {
        payments::transfer_amount_measured(
            env,
            asset,
            &controller,
            &accumulator,
            received,
            GenericError::AmountMustBePositive,
        );

        // Record the measured pool receipt used as the outbound transfer amount.
        events::ClaimRevenueEvent {
            hub_id: hub_asset.hub_id,
            asset: asset.clone(),
            caller: caller.clone(),
            accumulator,
            amount: received,
        }
        .publish(env);
    }

    received
}
