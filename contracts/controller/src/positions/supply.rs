//! Supply flow: deposits collateral, creating the account when `account_id == 0`.
//!
//! Pipeline: auth → aggregate → cache → [account resolution] → configs →
//! validate → settle → persist → emit. Supply uses
//! `OraclePolicy::RiskDecreasing`; deposits cannot worsen account health, so
//! no LTV, health, or min-collateral gates run at the entrypoint.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountPositionType, Payment, PoolSupplyEntry, PositionMode,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Vec};
use stellar_macros::when_not_paused;

use super::{finalize_position_flow, AggregatedConfigs, AggregatedPayments, PositionSides};
use crate::cache::Cache;
use crate::emode;
use crate::events;
use crate::external::pool::pool_supply_call;
use crate::helpers;
use crate::helpers::{refresh_supply_risk_params, update_or_remove_supply_position};
use crate::oracle::policy::OraclePolicy;
use crate::positions::make_pool_action;
use crate::{helpers::utils, storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<(Address, i128)>,
    ) -> u64 {
        process_supply(&env, &caller, account_id, e_mode_category, &assets)
    }
}

/// Supplies one or more assets, creating an account when `account_id == 0`.
///
/// Duplicate assets are aggregated before pool calls. The controller stores
/// scaled supply shares returned by pools and emits one position/market batch.
pub fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let aggregated = utils::aggregate_positive_payments(env, assets);
    let mut cache = Cache::new(env, OraclePolicy::RiskDecreasing);
    let (acct_id, mut account) = resolve_supply_account(
        env,
        caller,
        account_id,
        e_mode_category,
        &aggregated,
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

fn resolve_supply_account(
    env: &Env,
    caller: &Address,
    account_id: u64,
    e_mode_category: u32,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> (u64, Account) {
    validation::require_non_empty_payments(env, aggregated);

    if account_id == 0 {
        create_account_for_first_asset(env, caller, e_mode_category, aggregated, cache)
    } else {
        let account = storage::get_account(env, account_id);
        // Zero is the unspecified sentinel; any non-zero value must match the
        // account's stored mode.
        if e_mode_category != 0 && e_mode_category != account.e_mode_category_id {
            panic_with_error!(env, common::errors::EModeError::EModeMismatch);
        }
        (account_id, account)
    }
}

/// Applies deduped positive deposits to an account.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let configs = AggregatedConfigs::resolve(env, account, aggregated, cache);

    validate_deposit(env, account, aggregated, &configs, cache);
    settle_deposit(env, caller, account, aggregated, &configs, cache);
}

fn validate_deposit(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    configs: &AggregatedConfigs,
    cache: &mut Cache,
) {
    validation::validate_bulk_position_limits(
        env,
        account,
        AccountPositionType::Deposit,
        aggregated,
    );

    for (asset, _) in aggregated {
        validation::require_market_active(env, cache, &asset);

        let asset_config = configs.get(env, &asset);

        emode::validate_e_mode_asset(env, cache, account.e_mode_category_id, &asset);

        assert_with_error!(
            env,
            asset_config.can_supply(),
            CollateralError::NotCollateral
        );
    }
}

fn settle_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    configs: &AggregatedConfigs,
    cache: &mut Cache,
) {
    // One pool call for the whole batch (one cross-contract frame); results
    // align with entries by index.
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<PoolSupplyEntry> = Vec::new(env);
    for (asset, amount_in) in aggregated {
        let asset_config = configs.get(env, &asset);
        utils::transfer_amount(
            env,
            &asset,
            caller,
            &pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&asset, &asset_config);
        entries.push_back(PoolSupplyEntry {
            action: make_pool_action(&position, amount_in, asset.clone()),
            supply_cap: asset_config.supply_cap,
        });
    }
    let results = pool_supply_call(env, &pool_addr, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        let asset = &entry.action.asset;
        let asset_config = configs.get(env, asset);

        let mut position = account.get_or_create_supply_position(asset, &asset_config);
        refresh_supply_risk_params(env, cache, account, asset, &mut position, &asset_config);
        // Merge ONLY the scaled share back; the pool does not echo collateral
        // risk params, so preserve the ones the controller holds.
        position.scaled_amount = Ray::from(result.position.scaled_amount_ray);

        // Cache the pool-returned index so post-action valuation reads it
        // instead of asking the pool again.
        cache.put_market_index(asset, &result.market_index);

        // Emit with the exact supply index the pool used, not a re-read.
        cache.record_position_update(
            events::PositionAction::Supply,
            asset,
            result.market_index.supply_index_ray,
            entry.action.amount,
            &position,
        );

        // Storage is written once after the whole supply batch completes.
        update_or_remove_supply_position(account, asset, &position);
    }
}

fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    _aggregated: &AggregatedPayments,
    _cache: &mut Cache,
) -> (u64, Account) {
    helpers::create_account(env, caller, e_mode_category, PositionMode::Normal)
}
