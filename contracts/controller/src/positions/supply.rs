use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, HubAssetKey, HubPayment,
    PoolAction, PoolPositionMutation, PoolSupplyEntry, PoolWithdrawEntry, PositionMode,
};
use soroban_sdk::{assert_with_error, vec, Address, Env, Vec};

use crate::account::{self, require_owner_or_delegate, update_or_remove_supply_position};
use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Context;
use crate::events;
use crate::external::pool::{pool_supply_call, pool_withdraw_call};
use crate::payments;
use crate::positions::require_external_recipient;
use crate::positions::{
    apply_leg_usage, enforce_post_pool_solvency, enforce_spoke_asset_flags, finalize_position_flow,
    for_each_leg, get_supply_position_or_panic, make_pool_action, validate_position_entry_gates,
    AggregatedPayments, FreezePolicy, LegDirection, LegOutcome, PositionSides,
};
use crate::risk::{refresh_supply_risk_params, validation, RiskRefreshScope};
use crate::spoke_usage::UsageSide;
use crate::storage;
use common::validation::expect_invariant;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WithdrawKind {
    Normal,
    Liquidation,
}

pub(crate) struct WithdrawalRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
}

/// Supplies collateral, creating an account when `account_id` is zero.
/// Third parties may only add to existing supply positions. Returns the account id.
pub(crate) fn process_supply(
    env: &Env,
    caller: &Address,
    account_id: u64,
    spoke_id: u32,
    assets: &Vec<HubPayment>,
) -> u64 {
    validation::require_authorized_caller(env, caller);
    let aggregated = payments::aggregate_positive_payments(env, assets);
    let mut cache = Context::new(env);

    let (acct_id, mut account) = account::load_or_create_account(
        env,
        caller,
        account_id,
        spoke_id,
        PositionMode::Normal,
        account::AccountGuard::Supply,
        &mut cache,
    );

    require_third_party_existing_supply(env, account_id, acct_id, caller, &account, &aggregated);

    process_deposit(env, caller, &mut account, &aggregated, &mut cache);

    finalize_position_flow(
        env,
        acct_id,
        &account,
        &mut cache,
        PositionSides::Supply,
        false,
    );
    acct_id
}

/// Restricts third parties to existing supply positions. New accounts are
/// exempt because the caller becomes their owner.
fn require_third_party_existing_supply(
    env: &Env,
    account_id: u64,
    resolved_account_id: u64,
    caller: &Address,
    account: &Account,
    aggregated: &AggregatedPayments,
) {
    if account_id != 0
        && !account::is_owner_or_delegate(env, resolved_account_id, caller, &account.owner)
    {
        for (hub_asset, _) in aggregated.iter() {
            assert_with_error!(
                env,
                account.supply_positions.contains_key(hub_asset.clone()),
                GenericError::NotAuthorized
            );
        }
    }
}

/// Checks supply entry gates and credits measured pool receipts.
pub(crate) fn process_deposit(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Context,
) {
    validate_position_entry_gates(
        env,
        account,
        aggregated,
        cache,
        AccountPositionType::Deposit,
    );
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<PoolSupplyEntry> = Vec::new(env);
    for (hub_asset, amount_in) in aggregated.iter() {
        let asset_config: AssetConfig = cache.require_spoke_asset(account.spoke_id, &hub_asset);
        let received = payments::transfer_amount_measured(
            env,
            &hub_asset.asset,
            caller,
            &pool_addr,
            amount_in,
            GenericError::AmountMustBePositive,
        );
        let position = account.get_or_create_supply_position(&hub_asset, &asset_config);
        entries.push_back(PoolSupplyEntry {
            action: make_pool_action(&position, received, hub_asset.clone()),
        });
    }

    let results = pool_supply_call(env, &pool_addr, &entries);
    for_each_leg(env, &entries, &results, |entry, result| {
        merge_supply_leg(env, account, &entry.action, &result, cache);
    });
}

/// Withdraws for an authorized owner/delegate and checks post-pool solvency.
/// Zero requests withdraw all; returns the pool's actual payouts per asset.
pub(crate) fn process_withdraw(
    env: &Env,
    caller: &Address,
    account_id: u64,
    withdrawals: &Vec<HubPayment>,
    to: Option<Address>,
) -> Vec<HubPayment> {
    validation::require_authorized_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Context::new(env);
    require_external_recipient(env, &mut cache, &recipient);
    let aggregated = payments::aggregate_payments(env, withdrawals, payments::ZeroLeg::MeansAll);

    let paid = settle_withdraw(env, &mut account, &recipient, &aggregated, &mut cache);
    let _ = enforce_post_pool_solvency(env, &mut cache, &mut account);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Supply,
        true,
    );
    paid
}

/// Enforces exit flags and withdraws the batch, treating zero as withdraw-all.
/// Returns each asset's actual pool payout.
fn settle_withdraw(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Context,
) -> Vec<HubPayment> {
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
        let requested = if amount == 0 {
            WITHDRAW_ALL_SENTINEL
        } else {
            amount
        };
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, requested, hub_asset.clone()),
            protocol_fee: 0,
        });
    }

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
    for_each_leg(env, &entries, &results, |entry, result| {
        paid.push_back((entry.action.hub_asset, result.actual_amount));
    });
    paid
}

/// Withdraws a resolved position under exit flags and returns the merged mutation.
pub(crate) fn execute_withdrawal(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    action: events::PositionAction,
    req: WithdrawalRequest<'_>,
    cache: &mut Context,
) -> PoolPositionMutation {
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
        recipient,
        WithdrawKind::Normal,
        action,
        &entries,
        cache,
    );
    expect_invariant(env, results.get(0))
}

/// Withdraws in normal or liquidation mode and merges the pool mutations.
pub(crate) fn apply_withdraw_batch(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    kind: WithdrawKind,
    action: events::PositionAction,
    entries: &Vec<PoolWithdrawEntry>,
    cache: &mut Context,
) -> Vec<PoolPositionMutation> {
    let is_liquidation = kind == WithdrawKind::Liquidation;
    let pool_addr = cache.cached_pool_address();
    let results = pool_withdraw_call(env, &pool_addr, recipient, is_liquidation, entries);
    for_each_leg(env, entries, &results, |entry, result| {
        let hub_asset = entry.action.hub_asset;
        let outcome = LegOutcome::from(&result);
        merge_withdraw_leg(env, account, action, &hub_asset, kind, &outcome, cache);
    });
    results
}

/// Merges a supply result, refreshing risk parameters and updating usage,
/// market index, and event state.
pub(crate) fn merge_supply_leg(
    env: &Env,
    account: &mut Account,
    action: &PoolAction,
    result: &PoolPositionMutation,
    cache: &mut Context,
) {
    let hub_asset = &action.hub_asset;
    let asset_config: AssetConfig = cache.require_spoke_asset(account.spoke_id, hub_asset);

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
        action.amount,
        &position,
    );

    update_or_remove_supply_position(account, hub_asset, &position);
}

/// Merges a withdrawal result, usage, market index, and event state.
/// Risk parameters refresh only for listed, nonempty positions outside liquidation.
pub(crate) fn merge_withdraw_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    kind: WithdrawKind,
    outcome: &LegOutcome,
    cache: &mut Context,
) {
    // Decide refresh eligibility before mutating the position.
    let may_restamp =
        leg_may_restamp_risk_params(kind, cache, account, hub_asset, outcome.new_scaled);
    let mut position = get_supply_position_or_panic(env, account, hub_asset);
    let old_scaled = position.scaled_amount;

    position.scaled_amount = outcome.new_scaled;
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

    if may_restamp && position.scaled_amount != Ray::ZERO {
        let config: AssetConfig = cache.require_spoke_asset(account.spoke_id, hub_asset);
        refresh_supply_risk_params(
            env,
            cache,
            account,
            hub_asset,
            &mut position,
            &config,
            RiskRefreshScope::FullTuple,
        );
    }

    update_or_remove_supply_position(account, hub_asset, &position);
    cache.record_supply_position_update(
        action,
        hub_asset,
        outcome.market_index.supply_index,
        outcome.amount,
        &position,
    );
}

fn leg_may_restamp_risk_params(
    kind: WithdrawKind,
    cache: &mut Context,
    account: &Account,
    hub_asset: &HubAssetKey,
    new_scaled: Ray,
) -> bool {
    kind != WithdrawKind::Liquidation
        && cache
            .cached_spoke_asset(account.spoke_id, hub_asset)
            .is_some()
        && new_scaled != Ray::ZERO
}
