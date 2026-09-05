use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPositionType, DebtPosition, HubAssetKey, PoolAction, PoolBorrowEntry,
    PoolPositionMutation,
};
use soroban_sdk::{assert_with_error, token, vec, Address, Env, Vec};

use crate::account::{self, require_owner_or_delegate};
use crate::context::Context;
use crate::events;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call, pool_repay_call};
use crate::payments;
use crate::positions::require_external_recipient;
use crate::positions::{
    apply_leg_usage, enforce_post_pool_solvency, enforce_spoke_asset_flags, finalize_position_flow,
    for_each_leg, get_debt_position_or_panic, make_pool_action, validate_position_entry_gates,
    AggregatedPayments, FreezePolicy, HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::risk::validation;
use crate::spoke_usage::UsageSide;
use crate::storage;
use common::validation::{expect_invariant, require_positive_amount};

pub(crate) struct RepaymentRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

/// Borrows to `to` or the authorized owner/delegate, then checks solvency.
/// Persists supply alongside debt when the check restamps supply LTVs.
pub(crate) fn process_borrow(
    env: &Env,
    caller: &Address,
    account_id: u64,
    borrows: &Vec<HubPayment>,
    to: Option<Address>,
) {
    validation::require_authorized_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Context::new(env);
    require_external_recipient(env, &mut cache, &recipient);
    let aggregated = payments::aggregate_positive_payments(env, borrows);

    validate_position_entry_gates(
        env,
        &account,
        &aggregated,
        &mut cache,
        AccountPositionType::Borrow,
    );
    settle_borrow(env, &mut account, &recipient, &aggregated, &mut cache);

    let restamped = enforce_post_pool_solvency(env, &mut cache, &mut account);
    let sides = if restamped {
        PositionSides::Both
    } else {
        PositionSides::Debt
    };
    finalize_position_flow(env, account_id, &account, &mut cache, sides, false);
}

/// Repays with the caller's measured transfers; loads and persists debt only.
pub(crate) fn process_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    payments_in: &Vec<HubPayment>,
) {
    validation::require_authorized_caller(env, caller);

    let aggregated = payments::aggregate_positive_payments(env, payments_in);
    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Context::new(env);

    settle_repay(env, &mut account, caller, &aggregated, &mut cache);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Debt,
        false,
    );
}

/// Borrows the aggregated amounts to `recipient` and merges the debt results.
fn settle_borrow(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Context,
) {
    let pool_addr = cache.cached_pool_address();
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        let position = account.get_or_create_debt_position(&hub_asset);
        entries.push_back(PoolBorrowEntry {
            action: make_pool_action(&position, amount, hub_asset.clone()),
        });
    }
    let results = pool_borrow_call(env, &pool_addr, recipient, &entries);
    for_each_leg(env, &entries, &results, |entry, result| {
        merge_debt_leg(
            env,
            account,
            events::PositionAction::Borrow,
            &entry.action.hub_asset,
            LegDirection::Entry {
                asset_decimals: result.asset_decimals,
            },
            &LegOutcome::from(&result),
            cache,
        );
    });
}

/// Funds the pool with measured transfers and repays the corresponding debts.
fn settle_repay(
    env: &Env,
    account: &mut Account,
    payer: &Address,
    aggregated: &AggregatedPayments,
    cache: &mut Context,
) {
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for (hub_asset, amount) in aggregated.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &hub_asset,
            FreezePolicy::AllowOnExit,
        );
        let position = get_debt_position_or_panic(env, account, &hub_asset);
        let amount_in = payments::transfer_amount_measured(
            env,
            &hub_asset.asset,
            payer,
            &pool_addr,
            amount,
            GenericError::AmountMustBePositive,
        );
        actions.push_back(make_pool_action(&position, amount_in, hub_asset.clone()));
    }
    apply_repay_batch(
        env,
        account,
        payer,
        events::PositionAction::Repay,
        &actions,
        cache,
    );
}

/// Merges debt, spoke usage, market index, and event state from a pool result.
/// Entries may open positions; exits require an existing position.
pub(crate) fn merge_debt_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    direction: LegDirection,
    outcome: &LegOutcome,
    cache: &mut Context,
) {
    let old_scaled = match direction {
        LegDirection::Entry { .. } => account
            .borrow_positions
            .get(hub_asset.clone())
            .map_or(Ray::ZERO, |p| Ray::from(p.scaled_amount)),
        LegDirection::Exit => get_debt_position_or_panic(env, account, hub_asset).scaled_amount,
    };
    let position = DebtPosition {
        scaled_amount: outcome.new_scaled,
    };

    apply_leg_usage(
        env,
        cache,
        account.spoke_id,
        UsageSide::Borrow,
        hub_asset,
        direction,
        old_scaled,
        outcome,
    );
    cache.put_market_index(hub_asset, &outcome.market_index);
    cache.record_debt_position_update(
        action,
        hub_asset,
        outcome.market_index.borrow_index,
        outcome.amount,
        &position,
    );
    account::update_or_remove_debt_position(account, hub_asset, &position);
}

/// Repays prefunded actions and merges the pool results. The pool refunds
/// excess to `payer`; returns the mutations.
pub(crate) fn apply_repay_batch(
    env: &Env,
    account: &mut Account,
    payer: &Address,
    action: events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Context,
) -> Vec<PoolPositionMutation> {
    let pool_addr = cache.cached_pool_address();
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for_each_leg(env, actions, &results, |entry, result| {
        merge_debt_leg(
            env,
            account,
            action,
            &entry.hub_asset,
            LegDirection::Exit,
            &LegOutcome::from(&result),
            cache,
        );
    });
    results
}

/// Repays a resolved debt from funds already received by the pool, enforcing
/// exit flags. Refunds excess to `refund_to` and returns the merged mutation.
pub(crate) fn repay_prefunded_position(
    env: &Env,
    account: &mut Account,
    refund_to: &Address,
    action: events::PositionAction,
    req: RepaymentRequest<'_>,
    cache: &mut Context,
) -> PoolPositionMutation {
    enforce_spoke_asset_flags(
        env,
        cache,
        account.spoke_id,
        req.hub_asset,
        FreezePolicy::AllowOnExit,
    );
    let actions = vec![
        env,
        make_pool_action(req.position, req.amount, req.hub_asset.clone()),
    ];
    let results = apply_repay_batch(env, account, refund_to, action, &actions, cache);
    expect_invariant(env, results.get(0))
}

/// Validates entry gates and borrows into the controller for a strategy.
/// Returns the measured receipt, net of any charged flash fee.
pub(crate) fn borrow_into_controller(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    charge_fee: bool,
    action: events::PositionAction,
    cache: &mut Context,
) -> i128 {
    require_positive_amount(env, amount);
    let aggregated = vec![env, (hub_debt.clone(), amount)];
    validate_position_entry_gates(
        env,
        account,
        &aggregated,
        cache,
        AccountPositionType::Borrow,
    );

    let position = account.get_or_create_debt_position(hub_debt);
    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&position, amount, hub_debt.clone());
    let controller = env.current_contract_address();
    let before = token::Client::new(env, &hub_debt.asset).balance(&controller);
    // Block token-hook reentry during funding, before the strategy swap guard.
    let result = storage::with_flash_guard(env, || {
        pool_create_strategy_call(env, &pool_addr, &controller, pool_action, charge_fee)
    });
    let measured = payments::balance_delta_since(env, &hub_debt.asset, &controller, before);
    assert_with_error!(
        env,
        measured == result.amount_received,
        GenericError::InternalError
    );
    assert_with_error!(env, measured > 0, GenericError::AmountMustBePositive);
    let mutation = PoolPositionMutation::from(&result);
    merge_debt_leg(
        env,
        account,
        action,
        hub_debt,
        LegDirection::Entry {
            asset_decimals: mutation.asset_decimals,
        },
        &LegOutcome::from(&mutation),
        cache,
    );
    measured
}
