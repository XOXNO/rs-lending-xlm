//! Non-position controller entrypoints.

use crate::account;
use crate::events::{CreateMarketEvent, UpdateMarketParamsEvent};
use crate::risk;
use common::errors::{CollateralError, GenericError, OracleError};
use common::math::fp::Wad;
use common::types::{
    AccountPosition, AssetConfig, HubAssetKey, InterestRateModel, MarketParamsRaw,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Vec};
use stellar_macros::{only_owner, when_not_paused};

use crate::context::Cache;
use crate::events;
use crate::external::pool::{
    pool_add_rewards_call, pool_claim_revenue_call, pool_create_market_call,
    pool_update_indexes_call, pool_update_params_call, pool_upgrade_call,
};
use crate::external::sac::sac_transfer_call;
use crate::risk::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::{
    payments as utils, risk::validation, storage, Controller, ControllerArgs, ControllerClient,
};

/// Deterministic salt for the one-time central pool deployment; the pool
/// address derives from (controller address, salt).
const POOL_DEPLOY_SALT: [u8; 32] = [0u8; 32];

#[contractimpl]
impl Controller {
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `PoolNotInitialized` - a listed market has not been created.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    #[when_not_paused]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);

        let mut cache = Cache::new(&env);
        let pool_addr = cache.cached_pool_address();
        for hub_asset in assets {
            // The pool owns the authoritative market record and reverts
            // `PoolNotInitialized` for an uncreated market.
            pool_update_indexes_call(&env, &pool_addr, &hub_asset);
        }
    }

    /// Extends the account's storage TTL. Callable by the account owner.
    ///
    /// # Errors
    /// * `AccountNotInMarket` - `caller` is not the account owner.
    pub fn renew_account(env: Env, caller: Address, account_id: u64) {
        storage::renew_controller_instance(&env);
        account::renew_account(&env, &caller, account_id);
    }

    /// Registers `delegate` as a manager that may act on `account_id`. Effective
    /// only while `delegate` is also a registered, active position manager.
    ///
    /// # Arguments
    /// * `caller` - must be the account owner.
    ///
    /// # Errors
    /// * `AccountNotInMarket` - `caller` is not the account owner.
    pub fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        storage::renew_controller_instance(&env);
        account::set_account_delegate(&env, &caller, account_id, &delegate, true);
    }

    /// Revokes `delegate` from `account_id`.
    ///
    /// # Arguments
    /// * `caller` - must be the account owner.
    ///
    /// # Errors
    /// * `AccountNotInMarket` - `caller` is not the account owner.
    pub fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        storage::renew_controller_instance(&env);
        account::set_account_delegate(&env, &caller, account_id, &delegate, false);
    }

    /// Deploys the central liquidity pool once, owned by this controller. The
    /// pool address derives from `(controller address, salt)`.
    ///
    /// # Errors
    /// * `PoolAlreadyDeployed` - the pool has already been deployed.
    /// * `TemplateNotSet` - no pool Wasm template has been configured.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
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

    /// interest-rate and risk params.
    ///
    /// # Arguments
    /// * `asset` - must equal `params.asset_id` and be an approved token.
    ///
    /// # Errors
    /// * `WrongToken` - `asset` does not match `params.asset_id`.
    /// * Param validation and market-exists reverts propagate from the pool's
    ///   `create_market` (rate-model bounds, `AssetAlreadySupported`).
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address {
        validation::require_hub_active(&env, hub_id);

        assert_with_error!(&env, params.asset_id == asset, GenericError::WrongToken);

        let pool_address = storage::get_pool(&env);
        pool_create_market_call(&env, &pool_address, hub_id, &params);

        storage::renew_controller_instance(&env);

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
        .publish(&env);

        pool_address
    }

    ///
    /// # Errors
    /// * `PoolNotInitialized` - the target market has not been created.
    ///
    /// # Events
    /// * `UpdateMarketParamsEvent` - the new rate-model parameters.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn upgrade_liquidity_pool_params(
        env: Env,
        hub_asset: HubAssetKey,
        params: InterestRateModel,
    ) {
        upgrade_liquidity_pool_params(&env, &hub_asset, &params);
    }

    /// Upgrades the deployed pool contract to `new_wasm_hash`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - the pool has not been deployed.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        let pool_addr = storage::get_pool(&env);
        pool_upgrade_call(&env, &pool_addr, &new_wasm_hash);
    }

    /// accumulator. Returns the amount claimed per asset.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `NoAccumulator` - no revenue accumulator has been configured.
    /// * `PoolNotInitialized` - a listed market has not been created.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    #[when_not_paused]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);
        let mut results = Vec::new(&env);
        let mut cache = Cache::new(&env);
        for hub_asset in assets {
            let amount = claim_revenue_for_asset_with_cache(&env, &hub_asset, &mut cache);
            results.push_back(amount);
        }
        results
    }

    /// Transfers external supply rewards into one or more markets, raising each
    /// market's supply index for its suppliers.
    ///
    /// # Arguments
    /// * `rewards` - `(hub-asset, amount)` legs; amounts must be positive.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` - a leg amount is not strictly positive.
    /// * `PoolNotInitialized` - a target market has not been created.
    /// * `NoSuppliersToReward` - a target market has no suppliers to receive the reward.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    #[when_not_paused]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(HubAssetKey, i128)>) {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);
        let mut cache = Cache::new(&env);
        for (hub_asset, amount) in rewards {
            add_reward(&env, &caller, &hub_asset, amount, &mut cache);
        }
    }

    /// Propagates spoke risk params onto supply positions. Permissionless;
    /// HF gate blocks threshold raises that would drop an account below min HF.
    /// Delisted spoke members keep stamped params and are skipped.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `HealthFactorTooLow` - a threshold raise would push an account below the
    ///   minimum safe health factor.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * A position-batch event per updated account.
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
        // The cache is shared across the batch for its token-rooted memos
        // (prices, oracles, pool sync data); the per-spoke context is reset per
        // account so a batch may mix accounts from different spokes.
        let mut cache = Cache::new(&env);

        for account_id in account_ids {
            cache.reset_spoke_context();
            sync_account_thresholds(&env, account_id, has_risks, &mut cache);
        }
    }
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

fn claim_revenue_for_asset_with_cache(
    env: &Env,
    hub_asset: &HubAssetKey,
    cache: &mut Cache,
) -> i128 {
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));

    let pool_addr = cache.cached_pool_address();

    // `claim_revenue` reverts `PoolNotInitialized` for an uncreated market.
    let result = pool_claim_revenue_call(env, &pool_addr, hub_asset);
    let amount = result.actual_amount;

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

/// Transfers rewards into a pool and increases the supply index for suppliers.
pub(crate) fn add_reward(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    cache: &mut Cache,
) {
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

    // `add_rewards` reverts `PoolNotInitialized` for an uncreated market.
    pool_add_rewards_call(env, &pool_addr, hub_asset, amount);
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
        // Delisted assets keep their stamped params; skip them instead of
        // blocking the rest of the account. Deprecated spokes sync normally.
        let Some(spoke_config) = cache.cached_spoke_asset(account.spoke_id, &hub_asset) else {
            continue;
        };
        let asset_config = AssetConfig::from(&spoke_config);

        let position =
            validation::expect_invariant(env, account.supply_positions.get(hub_asset.clone()));
        let mut updated_pos = position;

        // Only the Bps risk fields are copied; the position's scaled share amount is unchanged.
        let cfg_lt = asset_config.liquidation_threshold.raw() as u32;
        let cfg_ltv = asset_config.loan_to_value.raw() as u32;
        let cfg_bonus = asset_config.liquidation_bonus.raw() as u32;
        let cfg_fees = asset_config.liquidation_fees.raw() as u32;
        if has_risks {
            updated_pos.liquidation_threshold = cfg_lt;
        } else {
            updated_pos.loan_to_value = cfg_ltv;
            updated_pos.liquidation_bonus = cfg_bonus;
            updated_pos.liquidation_fees = cfg_fees;
        }

        let updated = AccountPosition::from(&updated_pos);
        account::update_or_remove_supply_position(&mut account, &hub_asset, &updated);

        // amount = 0: parameter change only, no deposit or withdraw.
        let market_index = cache.cached_market_index(&hub_asset);
        cache.record_supply_position_update(
            events::PositionAction::ParamUpd,
            &hub_asset,
            market_index.supply_index.raw(),
            0,
            &updated,
        );
    }

    storage::set_supply_positions(env, account_id, &account.supply_positions);

    if has_risks {
        let hf = risk::calculate_account_risk_totals(
            env,
            cache,
            account.spoke_id,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor;
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }

    cache.emit_position_batch(account_id, &account);
}
