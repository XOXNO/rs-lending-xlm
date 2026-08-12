use common::errors::GenericError;
use common::types::{
    Account, AccountPositionType, DebtPosition, HubAssetKey, PoolAction, PoolBorrowEntry,
    PoolPositionMutation,
};
use soroban_sdk::{vec, Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call, pool_repay_call};
use crate::payments;
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, for_each_leg, get_debt_position_or_panic,
    make_pool_action, merge_debt_leg, require_position_caller, validate_position_entry_gates,
    AggregatedPayments, FreezePolicy, HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::storage;
use common::validation::expect_invariant;

pub(crate) struct RepaymentRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

pub(crate) fn process_borrow(
    env: &Env,
    caller: &Address,
    account_id: u64,
    borrows: &Vec<HubPayment>,
    to: Option<Address>,
) {
    require_position_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    crate::account::require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);
    let aggregated = payments::aggregate_positive_payments(env, borrows);

    validate_position_entry_gates(env, &account, &aggregated, &mut cache, AccountPositionType::Borrow);
    settle_debt(env, &mut account, &aggregated, &mut cache, DebtFlowKind::Borrow { recipient: &recipient });

    let restamped = crate::positions::enforce_post_pool_solvency(env, &mut cache, &mut account);
    let sides = if restamped { PositionSides::BOTH } else { PositionSides::DEBT };
    finalize_position_flow(env, account_id, &account, &mut cache, sides, false);
}

pub(crate) fn process_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    payments_in: &Vec<HubPayment>,
) {
    require_position_caller(env, caller);

    let aggregated = payments::aggregate_positive_payments(env, payments_in);
    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Cache::new(env);

    settle_debt(env, &mut account, &aggregated, &mut cache, DebtFlowKind::Repay { payer: caller, action: events::PositionAction::Repay });

    finalize_position_flow(env, account_id, &account, &mut cache, PositionSides::DEBT, false);
}

enum DebtFlowKind<'a> {
    Borrow { recipient: &'a Address },
    Repay { payer: &'a Address, action: events::PositionAction },
}

fn settle_debt(
    env: &Env,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
    kind: DebtFlowKind<'_>,
) {
    let pool_addr = cache.cached_pool_address();

    match kind {
        DebtFlowKind::Borrow { recipient } => {
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
                    LegDirection::Entry { asset_decimals: result.asset_decimals },
                    &LegOutcome::from(&result),
                    cache,
                );
            });
        }
        DebtFlowKind::Repay { payer, action } => {
            let mut actions: Vec<PoolAction> = Vec::new(env);
            for (hub_asset, amount) in aggregated.iter() {
                enforce_spoke_asset_flags(env, cache, account.spoke_id, &hub_asset, FreezePolicy::AllowOnExit);
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
            apply_repay_batch(env, account, payer, action, &actions, cache);
        }
    }
}

pub(crate) fn apply_repay_batch(
    env: &Env,
    account: &mut Account,
    payer: &Address,
    action: events::PositionAction,
    actions: &Vec<PoolAction>,
    cache: &mut Cache,
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

pub(crate) fn execute_repayment(
    env: &Env,
    account: &mut Account,
    ctx: EventContext,
    req: RepaymentRequest<'_>,
    cache: &mut Cache,
) -> PoolPositionMutation {
    let EventContext { counterparty, action } = ctx;

    enforce_spoke_asset_flags(env, cache, account.spoke_id, req.hub_asset, FreezePolicy::AllowOnExit);
    let actions = vec![env, make_pool_action(req.position, req.amount, req.hub_asset.clone())];
    let results = apply_repay_batch(env, account, &counterparty, action, &actions, cache);
    expect_invariant(env, results.get(0))
}

pub(crate) fn borrow_into_controller(
    env: &Env,
    account: &mut Account,
    hub_debt: &HubAssetKey,
    amount: i128,
    charge_fee: bool,
    action: events::PositionAction,
    cache: &mut Cache,
) -> i128 {
    let hub_debt = hub_debt.clone();
    let payments: AggregatedPayments = vec![env, (hub_debt.clone(), amount)];
    let aggregated = payments::aggregate_positive_payments(env, &payments);
    validate_position_entry_gates(env, account, &aggregated, cache, AccountPositionType::Borrow);

    let position = account.get_or_create_debt_position(&hub_debt);
    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&position, amount, hub_debt.clone());
    let result = pool_create_strategy_call(env, &pool_addr, &env.current_contract_address(), pool_action, charge_fee);
    let mutation = PoolPositionMutation::from(&result);
    merge_debt_leg(
        env,
        account,
        action,
        &hub_debt,
        LegDirection::Entry { asset_decimals: mutation.asset_decimals },
        &LegOutcome::from(&mutation),
        cache,
    );
    result.amount_received
}
