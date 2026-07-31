use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AssetConfig, HubAssetKey, PoolPositionMutation, PoolWithdrawEntry,
};
use soroban_sdk::{vec, Address, Env, Vec};

use crate::account::{require_owner_or_delegate, update_or_remove_supply_position};
use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::pool_withdraw_call;
use crate::payments;
use crate::positions::{
    apply_leg_usage, enforce_post_pool_solvency, enforce_spoke_asset_flags, finalize_position_flow,
    for_each_leg, get_supply_position_or_panic, make_pool_action, require_position_caller,
    AggregatedPayments, FreezePolicy, HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, RiskRefreshScope};
use crate::spoke::UsageSide;
use crate::storage;
use common::validation::expect_invariant;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WithdrawKind {
    Normal,

    Liquidation,
}

pub(crate) enum SpokeRefresh {
    Frozen,

    Refresh,
}

pub(crate) struct WithdrawalRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

pub(crate) fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<HubPayment>,
    to: Option<Address>,
) -> Vec<HubPayment> {
    require_position_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);

    let aggregated = payments::aggregate_payments(env, withdrawals, payments::ZeroLeg::MeansAll);

    let paid = settle_withdraw(env, &mut account, &recipient, &aggregated, &mut cache);

    let _ = enforce_post_pool_solvency(env, &mut cache, &mut account);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::SUPPLY,
        true,
    );

    paid
}

fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<HubPayment> {
    let entries = build_withdraw_entries(env, account, aggregated, cache);
    let results = apply_withdraw_batch(
        env,
        account,
        recipient,
        WithdrawKind::Normal,
        events::PositionAction::Withdraw,
        &entries,
        cache,
    );

    let mut paid: Vec<HubPayment> = Vec::new(env);
    for (entry, result) in entries.iter().zip(results.iter()) {
        paid.push_back((entry.action.hub_asset, result.actual_amount));
    }
    paid
}

fn build_withdraw_entries(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<PoolWithdrawEntry> {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &hub_asset,
            FreezePolicy::AllowOnExit,
        );
        let position = get_supply_position_or_panic(env, account, &hub_asset);
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(
                &position,
                resolve_withdraw_amount(amount),
                hub_asset.clone(),
            ),
            protocol_fee: 0,
        });
    }
    entries
}

fn resolve_withdraw_amount(amount: i128) -> i128 {
    if amount == 0 {
        WITHDRAW_ALL_SENTINEL
    } else {
        amount
    }
}

pub(crate) fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: WithdrawalRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext {
        counterparty,
        action,
    } = ctx;
    enforce_spoke_asset_flags(
        env,
        cache,
        account.spoke_id,
        req.hub_asset,
        FreezePolicy::AllowOnExit,
    );
    let entries = vec![
        env,
        PoolWithdrawEntry {
            action: make_pool_action(req.position, req.amount, req.hub_asset.clone()),
            protocol_fee: 0,
        },
    ];
    let results = apply_withdraw_batch(
        env,
        account,
        &counterparty,
        WithdrawKind::Normal,
        action,
        &entries,
        cache,
    );
    expect_invariant(env, results.get(0))
}

pub(crate) fn apply_withdraw_batch(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    kind: WithdrawKind,
    action: events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Cache,
) -> Vec<PoolPositionMutation> {
    let is_liquidation = kind == WithdrawKind::Liquidation;
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    for_each_leg(env, entries, &results, |entry, result| {
        let hub_asset = entry.action.hub_asset;
        let outcome = LegOutcome::from(&result);
        let refresh_spoke =
            spoke_refresh_for_leg(kind, cache, account, &hub_asset, outcome.new_scaled);
        merge_withdraw_leg(
            env,
            account,
            action,
            &hub_asset,
            &refresh_spoke,
            &outcome,
            cache,
        );
    });
    results
}

pub(crate) fn merge_withdraw_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    refresh_spoke: &SpokeRefresh,
    outcome: &LegOutcome,
    cache: &mut Cache,
) {
    let mut result_position = get_supply_position_or_panic(env, account, hub_asset);
    let old_scaled = result_position.scaled_amount;

    result_position.scaled_amount = outcome.new_scaled;

    cache.put_market_index(hub_asset, &outcome.market_index);
    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Supply,
        hub_asset,
        LegDirection::Exit,
        old_scaled,
        outcome,
    );

    if matches!(refresh_spoke, SpokeRefresh::Refresh) && result_position.scaled_amount != Ray::ZERO
    {
        let config: AssetConfig = (&cache.require_spoke_asset(account.spoke_id, hub_asset)).into();
        refresh_supply_risk_params(
            env,
            cache,
            account,
            hub_asset,
            &mut result_position,
            &config,
            RiskRefreshScope::FullTuple,
        );
    }

    update_or_remove_supply_position(account, hub_asset, &result_position);

    cache.record_supply_position_update(
        action,
        hub_asset,
        outcome.market_index.supply_index,
        outcome.amount,
        &result_position,
    );
}

pub(crate) fn spoke_refresh_for_leg(
    kind: WithdrawKind,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    new_scaled: Ray,
) -> SpokeRefresh {
    if kind == WithdrawKind::Liquidation {
        return SpokeRefresh::Frozen;
    }
    if cache
        .cached_spoke_asset(account.spoke_id, hub_asset)
        .is_none()
    {
        return SpokeRefresh::Frozen;
    }
    if new_scaled == Ray::ZERO {
        return SpokeRefresh::Frozen;
    }
    SpokeRefresh::Refresh
}
