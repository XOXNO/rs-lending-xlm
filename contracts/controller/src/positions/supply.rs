//! Supply flow: deposits collateral, creating the account when `account_id == 0`.
//!
//! Pipeline: auth → aggregate → cache → [account resolution] → configs →
//! validate → settle → persist → emit. Deposits cannot worsen account health,
//! so no LTV, health, or min-collateral gates run at the entrypoint.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use controller_interface::types::{
    Account, AccountPositionType, HubAssetKey, PoolSupplyEntry, PositionMode,
};
use soroban_sdk::{assert_with_error, contractimpl, Address, Env, Vec};
use stellar_macros::when_not_paused;

use super::{
    enforce_spoke_asset_flags, finalize_position_flow, AggregatedPayments, PositionSides,
};
use crate::cache::Cache;
use crate::spoke;
use crate::events;
use crate::external::pool::pool_supply_call;
use crate::helpers;
use crate::helpers::{refresh_supply_risk_params, update_or_remove_supply_position};
use crate::positions::{make_pool_action, HubPayment};
use crate::{helpers::utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
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
///
/// Duplicate assets are aggregated before pool calls. The controller stores
/// scaled supply shares returned by pools and emits one position/market batch.
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
    let (acct_id, mut account) = resolve_supply_account(
        env,
        caller,
        account_id,
        spoke_id,
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
    spoke_id: u32,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> (u64, Account) {
    validation::require_non_empty_payments(env, aggregated);

    helpers::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        PositionMode::Normal,
        helpers::AccountGuard::Supply,
        cache,
    )
}

/// Applies deduped positive deposits to an account.
pub fn process_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    validate_deposit(env, account, aggregated, cache);
    settle_deposit(env, caller, account, aggregated, cache);
}

fn validate_deposit(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    validation::validate_bulk_position_limits(
        env,
        account,
        AccountPositionType::Deposit,
        aggregated,
    );

    for (hub_asset, _) in aggregated {
        validation::require_hub_active(env, hub_asset.hub_id);
        validation::require_market_active(env, cache, &hub_asset);

        // Risk config comes from the account's spoke (the single source of
        // truth); reverts `AssetNotSupported` when unlisted there.
        let asset_config = spoke::effective_asset_config(cache, account.spoke_id, &hub_asset);

        spoke::validate_spoke_lists_asset(env, cache, account.spoke_id, &hub_asset);
        // Frozen blocks new supply; paused blocks every verb.
        enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, true);

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
    cache: &mut Cache,
) {
    // One pool call for the whole batch (one cross-contract frame); results
    // align with entries by index.
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<PoolSupplyEntry> = Vec::new(env);
    for (hub_asset, amount_in) in aggregated {
        let asset_config = spoke::effective_asset_config(cache, account.spoke_id, &hub_asset);
        utils::transfer_amount(
            env,
            &hub_asset.asset,
            caller,
            &pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&hub_asset, &asset_config);
        entries.push_back(PoolSupplyEntry {
            action: make_pool_action(&position, amount_in, hub_asset.clone()),
        });
    }
    let results = pool_supply_call(env, &pool_addr, &entries);

    for (i, entry) in entries.iter().enumerate() {
        let result = validation::expect_invariant(env, results.get(i as u32));
        let hub_asset = &entry.action.hub_asset;
        let asset_config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);

        let mut position = account.get_or_create_supply_position(hub_asset, &asset_config);
        let old_scaled = position.scaled_amount;
        refresh_supply_risk_params(env, cache, account, hub_asset, &mut position, &asset_config);
        // Merge ONLY the scaled share back; the pool does not echo collateral
        // risk params, so preserve the ones the controller holds.
        position.scaled_amount = Ray::from(result.position.scaled_amount);
        // Spoke-cap accounting needs the asset decimals; source them from the
        // active market's oracle config.
        let asset_decimals = cache.cached_asset_oracle(&hub_asset.asset).asset_decimals;
        if let Some(ctx) = cache.spoke_usage_mut(account.spoke_id) {
            // dimensional: both values are Ray<Share(asset, supply)>; supply adds usage.
            let delta = position.scaled_amount - old_scaled;
            ctx.apply_supply_after_pool(
                env,
                hub_asset,
                delta,
                &result.market_index,
                asset_decimals,
            );
        }

        // Cache the pool-returned index so post-action valuation reads it
        // instead of asking the pool again.
        cache.put_market_index(hub_asset, &result.market_index);

        // Emit with the exact supply index the pool used, not a re-read.
        cache.record_position_update(
            events::PositionAction::Supply,
            &hub_asset.asset,
            result.market_index.supply_index,
            entry.action.amount,
            &position,
        );

        // Storage is written once after the whole supply batch completes.
        update_or_remove_supply_position(account, hub_asset, &position);
    }
}
