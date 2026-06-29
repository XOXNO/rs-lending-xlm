//! Public controller entrypoints that are not position verbs or strategies.
//!
//! Holds market bootstrap, keeper index updates, revenue claiming, and
//! threshold propagation; pool and token calls go through `external`.

use crate::events::{CreateMarketEvent, UpdateMarketParamsEvent};
use common::errors::{CollateralError, GenericError, OracleError};
use common::math::fp::Wad;
use controller_interface::types::{
    AccountPosition, HubAssetKey, InterestRateModel, MarketParamsRaw,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Vec};
use stellar_macros::{only_owner, when_not_paused};

use crate::cache::Cache;
use crate::spoke;
use crate::events;
use crate::external::pool::{
    pool_add_rewards_call, pool_claim_revenue_call, pool_create_market_call, pool_update_caps_call,
    pool_update_indexes_call, pool_update_params_call, pool_upgrade_call,
};
use crate::external::sac::sac_transfer_call;
use crate::{
    helpers::{self, utils, THRESHOLD_UPDATE_MIN_HF_RAW},
    storage, validation, Controller, ControllerArgs, ControllerClient,
};

/// Deterministic salt for the one-time central pool deployment; the pool
/// address derives from (controller address, salt).
const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);

        let mut cache = Cache::new(&env);
        sync_market_indexes(&env, &mut cache, &assets);
    }

    pub fn renew_account(env: Env, caller: Address, account_id: u64) {
        storage::renew_controller_instance(&env);
        renew_account(&env, &caller, account_id);
    }

    /// Owner-only: opts `delegate` into acting on `account_id`. Effective only
    /// while `delegate` is also a registered, active position manager.
    pub fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        storage::renew_controller_instance(&env);
        set_account_delegate(&env, &caller, account_id, &delegate, true);
    }

    /// Owner-only: revokes `delegate` from `account_id`.
    pub fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        storage::renew_controller_instance(&env);
        set_account_delegate(&env, &caller, account_id, &delegate, false);
    }

    /// One-time deployment of the central liquidity pool owned by this
    /// controller. Panics PoolAlreadyDeployed on repeat calls.
    #[only_owner]
    pub fn deploy_pool(env: Env) -> Address {
        storage::renew_controller_instance(&env);

        assert_with_error!(
            &env,
            storage::try_get_pool(&env).is_none(),
            GenericError::PoolAlreadyDeployed
        );

        let wasm_hash = storage::get_pool_template(&env);
        let salt = BytesN::from_array(&env, &POOL_DEPLOY_SALT);
        let pool = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, (env.current_contract_address(),));

        storage::set_pool(&env, &pool);
        pool
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address {
        create_liquidity_pool(&env, hub_id, &asset, &params)
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(
        env: Env,
        hub_asset: HubAssetKey,
        params: InterestRateModel,
    ) {
        upgrade_liquidity_pool_params(&env, &hub_asset, &params);
    }

    #[only_owner]
    pub fn update_pool_caps(env: Env, hub_asset: HubAssetKey, supply_cap: i128, borrow_cap: i128) {
        update_pool_caps(&env, &hub_asset, supply_cap, borrow_cap);
    }

    #[only_owner]
    pub fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        let pool_addr = storage::get_pool(&env);
        pool_upgrade_call(&env, &pool_addr, &new_wasm_hash);
    }

    #[when_not_paused]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);
        claim_revenue(&env, assets)
    }

    #[when_not_paused]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(HubAssetKey, i128)>) {
        caller.require_auth();
        // Instance TTL is renewed by `Cache::new` inside `add_rewards_batch`.
        validation::require_not_flash_loaning(&env);
        add_rewards_batch(&env, &caller, rewards);
    }

    /// Permissionless risk-param fan-out.
    /// Any caller may propagate updates because the HF gate prevents risk increases.
    #[when_not_paused]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);

        // Propagates risk-param updates for each supplied asset on each account.
        let mut cache = Cache::new(&env);

        for account_id in account_ids {
            sync_account_thresholds(&env, account_id, has_risks, &mut cache);
        }
    }
}

// Pool sync results become the canonical market-state batch for indexers.
fn sync_market_indexes(env: &Env, cache: &mut Cache, hub_assets: &Vec<HubAssetKey>) {
    let pool_addr = cache.cached_pool_address();
    for hub_asset in hub_assets {
        // The pool owns the authoritative market record: `update_indexes`
        // reverts `PoolNotInitialized` for an uncreated (hub, asset).
        pool_update_indexes_call(env, &pool_addr, &hub_asset);
    }
}

/// Registers the asset's market on the central pool under `hub_id`. The market
/// record lives on the pool (`pool_create_market_call`, which reverts
/// `AssetAlreadySupported` on a duplicate (hub, asset)); the controller keeps no
/// listing shadow. The asset stays inactive (unpriceable) until
/// `set_market_oracle_config` writes its token-rooted `AssetOracle` entry, and
/// becomes usable on a spoke once `add_asset_to_spoke` lists it there. Consumes
/// the token approval.
pub fn create_liquidity_pool(
    env: &Env,
    hub_id: u32,
    asset: &Address,
    params: &MarketParamsRaw,
) -> Address {
    validation::require_hub_active(env, hub_id);

    assert_with_error!(
        env,
        storage::is_token_approved(env, asset),
        GenericError::TokenNotApproved
    );

    let pool_address = storage::get_pool(env);
    // dimensional: params carries Ray rates/utilization, Bps reserve factor, and Token(asset) caps.
    pool_create_market_call(env, &pool_address, hub_id, params);

    storage::renew_controller_instance(env);

    // dimensional: event fields preserve raw Ray rate/utilization and Bps reserve-factor inputs.
    CreateMarketEvent {
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

    storage::set_token_approved(env, asset, false);

    pool_address
}

/// Updates hub supply/borrow caps on the central pool for one `(hub_id, asset)`
/// market.
pub fn update_pool_caps(env: &Env, hub_asset: &HubAssetKey, supply_cap: i128, borrow_cap: i128) {
    // dimensional: supply_cap/borrow_cap are HubCap(asset, side) in Token(asset) base units.
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let mut cache = Cache::new(env);
    storage::renew_controller_instance(env);
    // The forward invariant (spoke cap <= hub cap) is enforced when each spoke
    // asset is configured. Spoke listings are not enumerable from an asset, so
    // the reverse check at cap-update time is dropped; at runtime the hub gate
    // (pool) and spoke gate bind independently, the tighter one winning. The
    // pool reverts `PoolNotInitialized` for an uncreated (hub, asset).
    let pool_addr = cache.cached_pool_address();
    pool_update_caps_call(env, &pool_addr, hub_asset, supply_cap, borrow_cap);
}

/// Accrues pool indexes before replacing the market's interest-rate model.
pub fn upgrade_liquidity_pool_params(
    env: &Env,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    let mut cache = Cache::new(env);
    storage::renew_controller_instance(env);

    let pool_addr = cache.cached_pool_address();

    // `update_indexes` reverts `PoolNotInitialized` for an uncreated market.
    pool_update_indexes_call(env, &pool_addr, hub_asset);

    // dimensional: params carries Ray rates/utilization and Bps reserve factor.
    pool_update_params_call(env, &pool_addr, hub_asset, params);

    // dimensional: event fields mirror the raw Ray and Bps governance update.
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

fn claim_revenue_for_asset_with_cache(
    env: &Env,
    hub_asset: &HubAssetKey,
    cache: &mut Cache,
) -> i128 {
    // `claim_revenue` reverts `PoolNotInitialized` for an uncreated market.
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));

    let pool_addr = cache.cached_pool_address();

    let result = pool_claim_revenue_call(env, &pool_addr, hub_asset);
    let amount = result.actual_amount;
    // dimensional: amount is Token(asset) revenue in asset-native units.

    if amount > 0 {
        sac_transfer_call(
            env,
            &hub_asset.asset,
            &env.current_contract_address(),
            &accumulator,
            &amount,
        );
    }

    amount
}

/// Claims protocol revenue per market and forwards SAC balances to the accumulator.
pub fn claim_revenue(
    env: &Env,
    hub_assets: soroban_sdk::Vec<HubAssetKey>,
) -> soroban_sdk::Vec<i128> {
    let mut results = soroban_sdk::Vec::new(env);
    let mut cache = Cache::new(env);
    for hub_asset in hub_assets.iter() {
        let amount = claim_revenue_for_asset_with_cache(env, &hub_asset, &mut cache);
        results.push_back(amount);
    }
    results
}

/// Transfers rewards into a pool and increases the supply index for suppliers.
pub fn add_reward(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) {
    // dimensional: amount is Token(asset) reward in asset-native units.
    // `add_rewards` reverts `PoolNotInitialized` for an uncreated market.
    validation::require_positive_amount(env, amount);

    let pool_addr = cache.cached_pool_address();

    utils::transfer_amount(
        env,
        &hub_asset.asset,
        caller,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    pool_add_rewards_call(env, &pool_addr, hub_asset, amount);
}

pub fn add_rewards_batch(env: &Env, caller: &Address, rewards: Vec<(HubAssetKey, i128)>) {
    let mut cache = Cache::new(env);
    for (hub_asset, amount) in rewards.iter() {
        add_reward(env, caller, &hub_asset, amount, &mut cache);
    }
}

pub fn renew_account(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);

    storage::renew_user_account(env, account_id);
}

/// Owner-only mutation of an account's delegate list. Only the account owner
/// manages delegates; a delegate cannot add or remove other delegates.
fn set_account_delegate(
    env: &Env,
    caller: &Address,
    account_id: u64,
    delegate: &Address,
    add: bool,
) {
    caller.require_auth();
    let meta = storage::get_account_meta(env, account_id);
    assert_with_error!(env, meta.owner == *caller, GenericError::AccountNotInMarket);

    if add {
        storage::add_delegate(env, account_id, delegate);
    } else {
        storage::remove_delegate(env, account_id, delegate);
    }
}

/// Syncs risk params on each supply position for one account, then runs a
/// single HF gate when `has_risks` propagates liquidation thresholds.
fn sync_account_thresholds(env: &Env, account_id: u64, has_risks: bool, cache: &mut Cache) {
    // No-op when the account is gone (bad-debt cleanup, full exit).
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_supply_positions(env, account_id);
    if supply_positions.is_empty() {
        return;
    }

    // Load borrow positions only when the health-factor gate requires them.
    let borrow_positions = if has_risks {
        storage::get_debt_positions(env, account_id)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    let mut account = storage::account_from_parts(meta, supply_positions, borrow_positions);
    let assets = account.supply_positions.keys();

    for hub_asset in assets.iter() {
        // `effective_asset_config` reverts `AssetNotSupported` when the held
        // asset is not listed on the account's spoke.
        let asset_config = spoke::effective_asset_config(env, account.spoke_id, &hub_asset);

        let position =
            validation::expect_invariant(env, account.supply_positions.get(hub_asset.clone()));
        let mut updated_pos = position;

        // dimensional: raw risk params are Bps snapshots; scaled_amount is unchanged.
        let cfg_lt = asset_config.liquidation_threshold.raw() as u32;
        let cfg_ltv = asset_config.loan_to_value.raw() as u32;
        let cfg_bonus = asset_config.liquidation_bonus.raw() as u32;
        if has_risks {
            updated_pos.liquidation_threshold = cfg_lt;
        } else {
            updated_pos.loan_to_value = cfg_ltv;
            updated_pos.liquidation_bonus = cfg_bonus;
        }

        let updated = AccountPosition::from(&updated_pos);
        helpers::update_or_remove_supply_position(&mut account, &hub_asset, &updated);

        // amount = 0: parameter change only, no deposit or withdraw.
        let market_index = cache.cached_market_index(&hub_asset);
        cache.record_position_update(
            events::PositionAction::ParamUpd,
            &hub_asset.asset,
            market_index.supply_index.raw(),
            0,
            &updated,
        );
    }

    storage::set_supply_positions(env, account_id, &account.supply_positions);

    if has_risks {
        let hf = helpers::calculate_account_risk_totals(
            env,
            cache,
            account.spoke_id,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor;
        // dimensional: hf and THRESHOLD_UPDATE_MIN_HF_RAW are WAD-scaled HealthFactor.
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }

    cache.emit_position_batch(account_id, &account);
}
