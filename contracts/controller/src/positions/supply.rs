//! User and strategy supply: transfer into the pool, mint scaled shares, stamp
//! controller-owned collateral risk params.
//!
//! Deposits never re-run post-pool solvency (unlike debt-bearing borrow/withdraw).
//! Auth and entry gates live on the public `supply` path; strategy callers reuse
//! `process_deposit` after their own auth. See `docs/reference/invariants.md` §3.

use common::errors::GenericError;
use common::math::fp::Ray;
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
    finalize_position_flow, make_pool_action, validate_position_entry_gates, AggregatedPayments,
    HubPayment, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, validation, RiskRefreshScope};
use crate::spoke::UsageSide;
use common::validation::expect_invariant;

/// Auth, aggregate, load/create account, deposit, then persist supply positions.
///
/// Does not enforce post-pool solvency. `remove_if_empty` is false so a brand-new
/// empty account is not cleaned up if the deposit path is ever a no-op.
pub(crate) fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    assets: &Vec<HubPayment>,
) -> u64 {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
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

    // Third parties may top up existing supply legs (gift collateral) but must
    // not open new asset slots — that would fill `max_supply_positions` and
    // grief the owner. Owner and active delegates retain full supply rights.
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

/// Entry gates then settle. Shared by `supply` and strategies that already hold
/// auth / flash-loan / account context (multiply, swap collateral, migrate).
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

/// Transfer tokens into the pool, one batch `pool.supply`, merge results.
fn settle_supply(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let entries = build_supply_entries(env, caller, account, aggregated, cache, &pool_addr);
    let results = pool_supply_call(env, &pool_addr, &entries);
    apply_supply_batch(env, account, &entries, &results, cache);
}

/// Moves each supply leg to the pool, then builds the matching `PoolSupplyEntry`.
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

/// Input-ordered pool results → `merge_supply_leg` per entry.
fn apply_supply_batch(
    env: &Env,
    account: &mut Account,
    entries: &Vec<PoolSupplyEntry>,
    results: &Vec<PoolPositionMutation>,
    cache: &mut Cache,
) {
    for (i, entry) in entries.iter().enumerate() {
        let result = expect_invariant(env, results.get(i as u32));
        merge_supply_leg(env, account, &entry, &result, cache);
    }
}

/// Per-leg merge: risk params, scaled shares, spoke usage, event, supply map.
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

    // Pool owns scaled shares; controller keeps collateral risk params.
    position.scaled_amount = Ray::from(result.position.scaled_amount);

    let delta = position.scaled_amount.checked_sub(env, old_scaled);
    cache.apply_spoke_entry(
        account.spoke_id,
        UsageSide::Supply,
        hub_asset,
        delta,
        &result.market_index,
        result.asset_decimals,
    );

    cache.put_market_index(hub_asset, &result.market_index);
    cache.record_supply_position_update(
        events::PositionAction::Supply,
        hub_asset,
        result.market_index.supply_index,
        entry.action.amount,
        &position,
    );

    update_or_remove_supply_position(account, hub_asset, &position);
}
