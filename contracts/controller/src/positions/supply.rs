use common::errors::GenericError;
use common::types::{
    Account, AccountPositionType, AssetConfig, PoolPositionMutation, PoolSupplyEntry, PositionMode,
};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use crate::account::{self, update_or_remove_supply_position};
use crate::context::Cache;
use crate::events;
use crate::external::pool::pool_supply_call;
use crate::payments;
use crate::positions::{
    apply_leg_usage, finalize_position_flow, for_each_leg, make_pool_action,
    require_position_caller, validate_position_entry_gates, AggregatedPayments, HubPayment,
    LegDirection, LegOutcome, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, RiskRefreshScope};
use crate::spoke::UsageSide;

pub(crate) fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    assets: &Vec<HubPayment>,
) -> u64 {
    require_position_caller(env, caller);
    let aggregated = payments::aggregate_positive_payments(env, assets);
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

    if account_id != 0 && !account::is_owner_or_delegate(env, acct_id, caller, &account.owner) {
        for (hub_asset, _) in aggregated.iter() {
            assert_with_error!(
                env,
                account.supply_positions.contains_key(hub_asset.clone()),
                GenericError::NotAuthorized
            );
        }
    }

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

pub(crate) fn process_deposit(
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
    settle_supply(env, caller, account, aggregated, cache);
}

fn settle_supply(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let entries = build_supply_entries(env, caller, account, aggregated, cache, &pool_addr);
    apply_supply_batch(env, account, &entries, cache);
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
        let asset_config: AssetConfig =
            (&cache.require_spoke_asset(account.spoke_id, &hub_asset)).into();
        let received = payments::transfer_amount_measured(
            env,
            &hub_asset.asset,
            caller,
            pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&hub_asset, &asset_config);
        entries.push_back(PoolSupplyEntry {
            action: make_pool_action(&position, received, hub_asset.clone()),
        });
    }
    entries
}

fn apply_supply_batch(
    env: &Env,
    account: &mut Account,
    entries: &Vec<PoolSupplyEntry>,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let results = pool_supply_call(env, &pool_addr, entries);
    for_each_leg(env, entries, &results, |entry, result| {
        merge_supply_leg(env, account, &entry, &result, cache);
    });
}

fn merge_supply_leg(
    env: &Env,
    account: &mut Account,
    entry: &PoolSupplyEntry,
    result: &PoolPositionMutation,
    cache: &mut Cache,
) {
    let hub_asset = &entry.action.hub_asset;
    let asset_config: AssetConfig =
        (&cache.require_spoke_asset(account.spoke_id, hub_asset)).into();

    let mut position = account.get_or_create_supply_position(hub_asset, &asset_config);
    let old_scaled = position.scaled_amount;

    refresh_supply_risk_params(
        env,
        cache,
        account,
        hub_asset,
        &mut position,
        &asset_config,
        RiskRefreshScope::FullTuple,
    );

    let outcome = LegOutcome::from(result);
    position.scaled_amount = outcome.new_scaled;

    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Supply,
        hub_asset,
        LegDirection::Entry {
            asset_decimals: result.asset_decimals,
        },
        old_scaled,
        &outcome,
    );

    cache.put_market_index(hub_asset, &outcome.market_index);
    cache.record_supply_position_update(
        events::PositionAction::Supply,
        hub_asset,
        outcome.market_index.supply_index,
        entry.action.amount,
        &position,
    );

    update_or_remove_supply_position(account, hub_asset, &position);
}
