//! Public controller entrypoints that are not position verbs or strategies.
//!
//! This module keeps market bootstrap, keeper index updates, revenue claiming,
//! and threshold propagation on the contract surface while
//! delegating pool and token calls through `cross_contract`.

use common::errors::{CollateralError, GenericError, OracleError};
use common::events::{
    emit_create_market, emit_update_market_params, CreateMarketEvent, UpdateMarketParamsEvent,
};
use common::math::fp::Wad;
use common::types::{
    AccountPosition, AccountPositionType, AssetConfig, AssetConfigRaw, InterestRateModel,
    MarketConfig, MarketOracleConfig, MarketParamsRaw, MarketStatus, PriceFeed,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, symbol_short, xdr::ToXdr, Address, BytesN,
    Env, Vec,
};
use stellar_macros::{only_owner, only_role, when_not_paused};

use crate::cache::ControllerCache;
use crate::cross_contract::pool::{
    pool_add_rewards_call, pool_claim_revenue_call, pool_update_indexes_call,
    pool_update_params_call, pool_upgrade_call,
};
use crate::cross_contract::sac::sac_transfer_call;
use crate::oracle::policy::OraclePolicy;
use crate::{helpers, storage, utils, validation, Controller, ControllerArgs, ControllerClient};

// Supported SAC decimal range for RAY/WAD conversions.
const MIN_ASSET_DECIMALS: u32 = 1;
const MAX_ASSET_DECIMALS: u32 = 18;
// Minimum post-update health factor for keeper threshold propagation.
const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

#[contractimpl]
impl Controller {
    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<Address>) {
        validation::require_not_flash_loaning(&env);

        let mut cache = ControllerCache::new(&env, OraclePolicy::RiskDecreasing);
        sync_market_indexes(&env, &mut cache, &assets);
        cache.emit_market_batch();
    }

    pub fn renew_account(env: Env, caller: Address, account_id: u64) {
        storage::renew_controller_instance(&env);
        renew_account(&env, &caller, account_id);
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
    ) -> Address {
        // Inner `create_liquidity_pool` already bumps the controller instance.
        create_liquidity_pool(&env, &asset, &params, &config)
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        storage::renew_controller_instance(&env);
        upgrade_liquidity_pool_params(&env, &asset, &params);
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool(env: Env, asset: Address, new_wasm_hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        upgrade_liquidity_pool(&env, &asset, new_wasm_hash);
    }

    #[when_not_paused]
    #[only_role(caller, "REVENUE")]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<Address>) -> Vec<i128> {
        // Instance TTL is renewed by `ControllerCache::new` inside `claim_revenue`.
        let _ = caller;
        validation::require_not_flash_loaning(&env);
        claim_revenue(&env, assets)
    }

    #[only_role(caller, "REVENUE")]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(Address, i128)>) {
        // Instance TTL is renewed by `ControllerCache::new` inside `add_rewards_batch`.
        validation::require_not_flash_loaning(&env);
        add_rewards_batch(&env, &caller, rewards);
    }

    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        asset: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        validation::require_not_flash_loaning(&env);

        // Propagates threshold updates with safety buffer.
        let risk = match has_risks {
            true => OraclePolicy::RiskIncreasing,
            false => OraclePolicy::RiskDecreasing,
        };
        let mut cache = ControllerCache::new(&env, risk);
        validation::require_asset_supported(&env, &mut cache, &asset);

        let base_config = cache.cached_asset_config(&asset);
        let price_feed = cache.cached_price(&asset);

        for account_id in account_ids {
            let mut account_asset_config = base_config.clone();

            update_position_threshold(
                &env,
                account_id,
                ThresholdUpdate {
                    asset: &asset,
                    has_risks,
                    asset_config: &mut account_asset_config,
                    feed: &price_feed,
                },
                &mut cache,
            );
        }
    }
}

// Pool sync results become the canonical market-state batch for indexers.
fn sync_market_indexes(env: &Env, cache: &mut ControllerCache, assets: &Vec<Address>) {
    for asset in assets {
        let pool_addr = cache.cached_pool_address(&asset);
        let state = pool_update_indexes_call(env, &pool_addr);
        // Refresh cache for subsequent reads.
        cache.record_market_update(&state);
    }
}

fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &AssetConfigRaw,
    _token_decimals: u32,
) {
    assert_with_error!(env, params.asset_id == *asset, GenericError::WrongToken);
    #[cfg(not(feature = "testing"))]
    assert_with_error!(
        env,
        params.asset_decimals == _token_decimals,
        GenericError::InvalidAsset
    );

    assert_with_error!(
        env,
        (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&params.asset_decimals),
        GenericError::InvalidAsset
    );

    validation::validate_asset_config(env, config);
    params.verify_rate_model(env);
}

/// Deploys a pool in `PendingOracle` state and consumes the token approval.
pub fn create_liquidity_pool(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &AssetConfigRaw,
) -> Address {
    let token_decimals = validation::validate_and_fetch_token_decimals(env, asset);

    assert_with_error!(
        env,
        !storage::has_market_config(env, asset),
        GenericError::AssetAlreadySupported
    );

    assert_with_error!(
        env,
        storage::is_token_approved(env, asset),
        GenericError::TokenNotApproved
    );

    validate_market_creation(env, asset, params, config, token_decimals);

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

    storage::set_token_approved(env, asset, false);

    pool_address
}

/// Accrues pool indexes before replacing the pool interest-rate model.
pub fn upgrade_liquidity_pool_params(env: &Env, asset: &Address, params: &InterestRateModel) {
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_asset_supported(env, &mut cache, asset);

    let pool_addr = cache.cached_pool_address(asset);

    params.verify(env);

    let state = pool_update_indexes_call(env, &pool_addr);
    cache.record_market_update(&state);
    cache.emit_market_batch();

    pool_update_params_call(env, &pool_addr, params);

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

/// Upgrades the deployed pool contract for `asset`.
pub fn upgrade_liquidity_pool(env: &Env, asset: &Address, new_wasm_hash: BytesN<32>) {
    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_asset_supported(env, &mut cache, asset);
    let pool_addr = cache.cached_pool_address(asset);
    pool_upgrade_call(env, &pool_addr, &new_wasm_hash);
}

fn claim_revenue_for_asset_with_cache(
    env: &Env,
    asset: &Address,
    cache: &mut ControllerCache,
) -> i128 {
    validation::require_asset_supported(env, cache, asset);

    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));

    let pool_addr = cache.cached_pool_address(asset);

    let result = pool_claim_revenue_call(env, &pool_addr);
    cache.record_market_update(&result.market_state);
    let amount = result.actual_amount;

    if amount > 0 {
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

/// Claims protocol revenue from each pool and forwards SAC balances to the accumulator.
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

/// Transfers rewards into a pool and increases the supply index for suppliers.
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

pub fn renew_account(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);

    storage::renew_user_account(env, account_id);
}

/// Per-account inputs for a keeper threshold propagation.
struct ThresholdUpdate<'a> {
    asset: &'a Address,
    has_risks: bool,
    asset_config: &'a mut AssetConfig,
    feed: &'a PriceFeed,
}

fn update_position_threshold(
    env: &Env,
    account_id: u64,
    update_req: ThresholdUpdate<'_>,
    cache: &mut ControllerCache,
) {
    let ThresholdUpdate {
        asset,
        has_risks,
        asset_config,
        feed,
    } = update_req;

    // No-op when the account is gone (bad-debt cleanup, full exit).
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_supply_positions(env, account_id);

    // No-op when the account has no supply position for this asset.
    let Some(position) = supply_positions.get(asset.clone()) else {
        return;
    };

    // Load borrow positions only when the health-factor gate requires them.
    let borrow_positions = if has_risks {
        storage::get_debt_positions(env, account_id)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    // Apply e-mode overrides.
    let e_mode_category = crate::emode::e_mode_category(env, meta.e_mode_category_id);
    let asset_emode_config = cache.cached_emode_asset(meta.e_mode_category_id, asset);
    crate::emode::apply_e_mode_to_asset_config(
        env,
        asset_config,
        &e_mode_category,
        asset_emode_config,
    );

    let mut updated_pos = position;

    let cfg_lt = asset_config.liquidation_threshold.raw() as u32;
    let cfg_ltv = asset_config.loan_to_value.raw() as u32;
    let cfg_bonus = asset_config.liquidation_bonus.raw() as u32;
    if has_risks {
        if updated_pos.liquidation_threshold_bps != cfg_lt {
            updated_pos.liquidation_threshold_bps = cfg_lt;
        }
    } else {
        if updated_pos.loan_to_value_bps != cfg_ltv {
            updated_pos.loan_to_value_bps = cfg_ltv;
        }
        if updated_pos.liquidation_bonus_bps != cfg_bonus {
            updated_pos.liquidation_bonus_bps = cfg_bonus;
        }
    }

    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);
    helpers::update_or_remove_supply_position(
        &mut account,
        asset,
        &AccountPosition::from(&updated_pos),
    );

    // Persist only the supply side; borrow stays as-is.
    storage::set_supply_positions(env, account_id, &account.supply_positions);

    // Enforce safety buffer on risky updates.
    if has_risks {
        let hf = helpers::calculate_health_factor(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        assert_with_error!(
            env,
            hf >= Wad::from_raw(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }

    // Record a position update with amount = 0; no deposit or withdraw
    // occurred, only a parameter change.
    let market_index = cache.cached_market_index(asset);
    cache.record_position_update(
        symbol_short!("param_upd"),
        AccountPositionType::Deposit,
        asset,
        market_index.supply_index.raw(),
        0,
        &AccountPosition::from(&updated_pos),
        Some(feed.price.raw()),
    );
    cache.emit_position_batch(account_id, &account);
}
