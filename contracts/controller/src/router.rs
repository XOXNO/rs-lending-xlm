use common::errors::{GenericError, OracleError};
use common::events::{
    emit_create_market, emit_update_market_params, CreateMarketEvent, UpdateMarketParamsEvent,
};
use common::types::{
    AssetConfigRaw, ControllerKey, InterestRateModel, MarketConfig, MarketOracleConfig,
    MarketParamsRaw, MarketStatus,
};
use soroban_sdk::{contractimpl, panic_with_error, token, xdr::ToXdr, Address, BytesN, Env, Vec};
use stellar_macros::{only_owner, only_role, when_not_paused};

use crate::cache::ControllerCache;
use crate::cross_contract::pool::{
    pool_add_rewards_call, pool_claim_revenue_call, pool_update_indexes_call,
};
use crate::cross_contract::sac::sac_transfer_call;
use crate::oracle::policy::OraclePolicy;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<Address>) {
        validation::require_not_flash_loaning(&env);

        let mut cache = ControllerCache::new(&env, OraclePolicy::RiskDecreasing);
        utils::sync_market_indexes(&env, &mut cache, &assets);
        cache.emit_market_batch();
    }

    pub fn renew_account(env: Env, caller: Address, account_id: u64) {
        renew_account(&env, &caller, account_id);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_shared_state(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        keepalive_shared_state(&env, &assets);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_accounts(env: Env, caller: Address, account_ids: Vec<u64>) {
        let _ = caller;
        keepalive_accounts(&env, &account_ids);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_pools(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        keepalive_pools(&env, &assets);
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
    ) -> Address {
        create_liquidity_pool(&env, &asset, &params, &config)
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        upgrade_liquidity_pool_params(&env, &asset, &params);
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool(env: Env, asset: Address, new_wasm_hash: BytesN<32>) {
        upgrade_liquidity_pool(&env, &asset, new_wasm_hash);
    }

    #[when_not_paused]
    #[only_role(caller, "REVENUE")]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<Address>) -> Vec<i128> {
        let _ = caller;
        validation::require_not_flash_loaning(&env);
        claim_revenue(&env, assets)
    }

    #[only_role(caller, "REVENUE")]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(Address, i128)>) {
        validation::require_not_flash_loaning(&env);
        add_rewards_batch(&env, &caller, rewards);
    }
}

// Valid asset decimal bounds.
const MIN_ASSET_DECIMALS: u32 = 1;
const MAX_ASSET_DECIMALS: u32 = 18;

fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &AssetConfigRaw,
    _token_decimals: u32,
) {
    if params.asset_id != *asset {
        panic_with_error!(env, GenericError::WrongToken);
    }
    #[cfg(not(feature = "testing"))]
    if params.asset_decimals != _token_decimals {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    if !(MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&params.asset_decimals) {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    validation::validate_asset_config(env, config);
    params.verify_rate_model(env);
}

// Deploys liquidity pool.
pub fn create_liquidity_pool(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &AssetConfigRaw,
) -> Address {
    let token_client = token::Client::new(env, asset);
    let token_decimals = match token_client.try_decimals() {
        Ok(Ok(d)) => d,
        _ => panic_with_error!(env, GenericError::InvalidAsset),
    };
    if !matches!(token_client.try_symbol(), Ok(Ok(_))) {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    if storage::has_market_config(env, asset) {
        panic_with_error!(env, GenericError::AssetAlreadySupported);
    }

    if !storage::is_token_approved(env, asset) {
        panic_with_error!(env, GenericError::TokenNotApproved);
    }

    validate_market_creation(env, asset, params, config, token_decimals);

    if !storage::has_pool_template(env) {
        panic_with_error!(env, GenericError::TemplateEmpty);
    }
    let wasm_hash = storage::get_pool_template(env);

    let salt = env.crypto().keccak256(&asset.to_xdr(env));

    let pool_address = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash, (env.current_contract_address(), params.clone()));

    let mut asset_config = config.clone();
    asset_config.e_mode_categories = soroban_sdk::Vec::new(env);
    let market = MarketConfig {
        status: MarketStatus::PendingOracle,
        asset_config,
        pool_address: pool_address.clone(),
        oracle_config: MarketOracleConfig::pending_for(asset.clone(), params.asset_decimals),
    };
    storage::set_market_config(env, asset, &market);

    // Tracks in pools list.
    storage::add_to_pools_list(env, asset);
    storage::renew_controller_instance(env);

    emit_create_market(
        env,
        CreateMarketEvent {
            base_asset: asset.clone(),
            max_borrow_rate: params.max_borrow_rate_ray,
            base_borrow_rate: params.base_borrow_rate_ray,
            slope1: params.slope1_ray,
            slope2: params.slope2_ray,
            slope3: params.slope3_ray,
            mid_utilization: params.mid_utilization_ray,
            optimal_utilization: params.optimal_utilization_ray,
            reserve_factor: params.reserve_factor_bps,
            market_address: pool_address.clone(),
            config: config.clone(),
        },
    );

    // Consume approval.
    storage::set_token_approved(env, asset, false);

    pool_address
}

// Upgrades pool interest-rate model.
pub fn upgrade_liquidity_pool_params(env: &Env, asset: &Address, params: &InterestRateModel) {
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_asset_supported(env, &mut cache, asset);

    let pool_addr = cache.cached_pool_address(asset);

    params.verify(env);

    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);

    let state = pool_update_indexes_call(env, &pool_addr);
    cache.record_market_update(&state);
    cache.emit_market_batch();

    pool_client.update_params(
        &params.max_borrow_rate_ray,
        &params.base_borrow_rate_ray,
        &params.slope1_ray,
        &params.slope2_ray,
        &params.slope3_ray,
        &params.mid_utilization_ray,
        &params.optimal_utilization_ray,
        &params.max_utilization_ray,
        &params.reserve_factor_bps,
    );

    emit_update_market_params(
        env,
        UpdateMarketParamsEvent {
            asset: asset.clone(),
            max_borrow_rate_ray: params.max_borrow_rate_ray,
            base_borrow_rate_ray: params.base_borrow_rate_ray,
            slope1_ray: params.slope1_ray,
            slope2_ray: params.slope2_ray,
            slope3_ray: params.slope3_ray,
            mid_utilization_ray: params.mid_utilization_ray,
            optimal_utilization_ray: params.optimal_utilization_ray,
            reserve_factor_bps: params.reserve_factor_bps,
        },
    );
}

// Upgrades pool WASM.
pub fn upgrade_liquidity_pool(env: &Env, asset: &Address, new_wasm_hash: BytesN<32>) {
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_asset_supported(env, &mut cache, asset);
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    pool_client.upgrade(&new_wasm_hash);
}

// Claims pool revenue.
fn claim_revenue_for_asset_with_cache(
    env: &Env,
    asset: &Address,
    cache: &mut ControllerCache,
) -> i128 {
    validation::require_asset_supported(env, cache, asset);

    if !storage::has_accumulator(env) {
        panic_with_error!(env, OracleError::NoAccumulator);
    }

    let pool_addr = cache.cached_pool_address(asset);

    let result = pool_claim_revenue_call(env, &pool_addr);
    cache.record_market_update(&result.market_state);
    let amount = result.actual_amount;

    if amount > 0 {
        let accumulator = storage::get_accumulator(env);
        sac_transfer_call(
            env,
            asset,
            &env.current_contract_address(),
            &accumulator,
            &amount,
        );
    }

    amount
}

// Claims revenue from multiple pools.
pub fn claim_revenue(env: &Env, assets: soroban_sdk::Vec<Address>) -> soroban_sdk::Vec<i128> {
    let mut results = soroban_sdk::Vec::new(env);
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        let amount = claim_revenue_for_asset_with_cache(env, &asset, &mut cache);
        results.push_back(amount);
    }
    cache.emit_market_batch();
    results
}

// Transfers rewards to pool and bumps index.
pub fn add_reward(
    env: &Env,
    caller: &Address,
    asset: &Address,
    amount: i128,
    cache: &mut ControllerCache,
) {
    validation::require_asset_supported(env, cache, asset);
    validation::require_amount_positive(env, amount);

    let pool_addr = cache.cached_pool_address(asset);

    let actual_received = utils::transfer_and_measure_received(
        env,
        asset,
        caller,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    let state = pool_add_rewards_call(env, &pool_addr, actual_received);
    cache.record_market_update(&state);
}

pub fn add_rewards_batch(env: &Env, caller: &Address, rewards: soroban_sdk::Vec<(Address, i128)>) {
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    for i in 0..rewards.len() {
        let (asset, amount) = validation::expect_invariant(env, rewards.get(i));
        add_reward(env, caller, &asset, amount, &mut cache);
    }
    cache.emit_market_batch();
}

pub fn keepalive_shared_state(env: &Env, assets: &soroban_sdk::Vec<Address>) {
    storage::renew_controller_instance(env);
    storage::renew_pools_list(env);

    let mut emode_categories = soroban_sdk::Vec::new(env);

    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        let market = match storage::try_get_market_config(env, &asset) {
            Some(m) => m,
            None => continue,
        };

        storage::renew_protocol_shared_key(env, &ControllerKey::Market(asset.clone()));
        storage::renew_isolated_debt_if_positive(env, &asset);

        // E-mode memberships dedupe.
        for category_id in market.asset_config.e_mode_categories.iter() {
            if !emode_categories.contains(category_id) {
                emode_categories.push_back(category_id);
            }
        }
    }

    for category_id in emode_categories {
        storage::renew_protocol_shared_key(env, &ControllerKey::EModeCategory(category_id));
    }
}

pub fn keepalive_accounts(env: &Env, account_ids: &soroban_sdk::Vec<u64>) {
    for i in 0..account_ids.len() {
        let account_id = validation::expect_invariant(env, account_ids.get(i));
        // renew_user_account is no-op if account missing.
        storage::renew_user_account(env, account_id);
    }
}

pub fn renew_account(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    if meta.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }

    storage::renew_user_account(env, account_id);
}

pub fn keepalive_pools(env: &Env, assets: &soroban_sdk::Vec<Address>) {
    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        if !storage::has_market_config(env, &asset) {
            continue;
        }
        let market = storage::get_market_config(env, &asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);
        pool_client.keepalive();
    }
}
