//! Supply flow. Deposits skip post-pool solvency gates.

use crate::account;
use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPositionType, HubAssetKey, PoolPositionMutation, PoolSupplyEntry, PositionMode,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};
use stellar_macros::when_not_paused;

use crate::account::update_or_remove_supply_position;
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_supply_call;
use crate::positions::{
    finalize_position_flow, validate_position_entry_gates, AggregatedPayments, PositionSides,
};
use crate::positions::{make_pool_action, HubPayment};
use crate::risk::refresh_supply_risk_params;
use crate::spoke;
use crate::{payments as utils, risk::validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Supplies one or more assets as collateral, opening a new account when
    /// `account_id == 0`. Returns the account id.
    ///
    /// # Arguments
    /// * `caller` - the account owner (or an active delegate for an existing
    ///   account); must authorize the call.
    /// * `account_id` - an existing account, or `0` to open a new one on `spoke_id`.
    /// * `assets` - `(hub-asset, amount)` deposit legs; amounts must be positive.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` - a leg amount is not strictly positive.
    /// * Entry gates: `HubNotActive`, `PairNotActive`, `AssetNotInSpoke`,
    ///   `SpokeAssetPaused`, `SpokeAssetFrozen`, `NotCollateral`, or
    ///   `PositionLimitExceeded`.
    /// * `SpokeSupplyCapReached` - the deposit would exceed the spoke supply cap.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * A position-batch event summarizing the account's updated supply legs.
    #[when_not_paused]
    pub fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        process_supply(&env, &caller, account_id, spoke_id, &assets)
    }
}

/// Supplies one or more assets, creating an account when `account_id == 0`.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    assets: &Vec<HubPayment>,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    let aggregated = utils::aggregate_positive_payments(env, assets);
    let mut cache = Cache::new(env);

    let (acct_id, mut account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        PositionMode::Normal,
        account::AccountGuard::Supply,
        &mut cache,
    );

    process_deposit(env, caller, &mut account, &aggregated, &mut cache);

    finalize_position_flow(
        env,
        acct_id,
        &account,
        &mut cache,
        PositionSides::SUPPLY,
        false,
    );

    acct_id
}

/// Applies deduped positive deposits to an account.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    validate_position_entry_gates(
        env,
        account,
        aggregated,
        cache,
        AccountPositionType::Deposit,
    );
    settle_deposit(env, caller, account, aggregated, cache);
}

fn settle_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    // One pool call for the whole batch; results align with entries by index.
    let pool_addr = cache.cached_pool_address();
    let entries = build_supply_entries(env, caller, account, aggregated, cache, &pool_addr);
    let results = pool_supply_call(env, &pool_addr, &entries);
    apply_supply_results(env, account, &entries, &results, cache);
}

fn build_supply_entries(
    env: &Env,
    caller: &Address,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
    pool_addr: &Address,
) -> Vec<PoolSupplyEntry> {
    let mut entries: Vec<PoolSupplyEntry> = Vec::new(env);
    for (hub_asset, amount_in) in aggregated {
        let asset_config = spoke::effective_asset_config(cache, account.spoke_id, &hub_asset);
        utils::transfer_amount(
            env,
            &hub_asset.asset,
            caller,
            pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&hub_asset, &asset_config);
        entries.push_back(PoolSupplyEntry {
            action: make_pool_action(&position, amount_in, hub_asset.clone()),
        });
    }
    entries
}

fn apply_supply_results(
    env: &Env,
    account: &mut Account,
    entries: &Vec<PoolSupplyEntry>,
    results: &Vec<PoolPositionMutation>,
    cache: &mut Cache,
) {
    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        let hub_asset = &entry.action.hub_asset;
        let asset_config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);

        let mut position = account.get_or_create_supply_position(hub_asset, &asset_config);
        let old_scaled = position.scaled_amount;
        refresh_supply_risk_params(env, cache, account, hub_asset, &mut position, &asset_config);

        // Merge only scaled share back; pool does not echo collateral risk params.
        position.scaled_amount = Ray::from(result.position.scaled_amount);

        let asset_decimals = cache.cached_asset_oracle(&hub_asset.asset).asset_decimals;
        let ctx = cache.require_spoke_usage_context(account.spoke_id);
        let delta = position.scaled_amount - old_scaled;
        ctx.apply_supply_after_pool(env, hub_asset, delta, &result.market_index, asset_decimals);

        cache.put_market_index(hub_asset, &result.market_index);
        cache.record_position_update(
            events::PositionAction::Supply,
            hub_asset,
            result.market_index.supply_index,
            entry.action.amount,
            &position,
        );

        update_or_remove_supply_position(account, hub_asset, &position);
    }
}
