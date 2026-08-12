
use common::types::{
    Account, AccountPositionType, HubAssetKey, PoolBorrowEntry, PoolPositionMutation,
};
use soroban_sdk::{vec, Address, Env, Vec};

use crate::account::require_owner_or_delegate;
use crate::context::Cache;
use crate::events;
use crate::external::pool::{pool_borrow_call, pool_create_strategy_call};
use crate::payments;
use crate::positions::{
    enforce_post_pool_solvency, finalize_position_flow, for_each_leg, make_pool_action,
    merge_debt_leg, require_position_caller, validate_position_entry_gates, AggregatedPayments,
    HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::storage;

pub(crate) fn process_borrow(
    env: &Env,
    caller: &Address,
    account_id: u64,
    borrows: &Vec<HubPayment>,
    to: Option<Address>,
) {
    require_position_caller(env, caller);

    let mut account = storage::get_account(env, account_id);
    require_owner_or_delegate(env, account_id, caller, &account.owner);

    let recipient = to.unwrap_or_else(|| caller.clone());
    let mut cache = Cache::new(env);
    let aggregated = payments::aggregate_positive_payments(env, borrows);

    validate_position_entry_gates(
        env,
        &account,
        &aggregated,
        &mut cache,
        AccountPositionType::Borrow,
    );
    settle_borrow(env, &recipient, &mut account, &aggregated, &mut cache);

    let restamped = enforce_post_pool_solvency(env, &mut cache, &mut account);
    let sides = if restamped {
        PositionSides::BOTH
    } else {
        PositionSides::DEBT
    };
    finalize_position_flow(env, account_id, &account, &mut cache, sides, false);
}

fn settle_borrow(
    env: &Env,
    recipient: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let entries = build_borrow_entries(env, account, aggregated);
    apply_borrow_batch(env, account, recipient, &entries, cache);
}

fn build_borrow_entries(
    env: &Env,
    account: &Account,
    aggregated: &AggregatedPayments,
) -> Vec<PoolBorrowEntry> {
    let mut entries: Vec<PoolBorrowEntry> = Vec::new(env);
    for (hub_asset, amount) in aggregated {
        let borrow_position = account.get_or_create_debt_position(&hub_asset);
        entries.push_back(PoolBorrowEntry {
            action: make_pool_action(&borrow_position, amount, hub_asset.clone()),
        });
    }
    entries
}

fn apply_borrow_batch(
    env: &Env,
    account: &mut Account,
    recipient: &Address,
    entries: &Vec<PoolBorrowEntry>,
    cache: &mut Cache,
) {
    let pool_addr = cache.cached_pool_address();
    let results = pool_borrow_call(env, &pool_addr, recipient, entries);
    for_each_leg(env, entries, &results, |entry, result| {
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
    validate_position_entry_gates(
        env,
        account,
        &aggregated,
        cache,
        AccountPositionType::Borrow,
    );

    let borrow_position = account.get_or_create_debt_position(&hub_debt);

    let pool_addr = cache.cached_pool_address();
    let pool_action = make_pool_action(&borrow_position, amount, hub_debt.clone());
    let result = pool_create_strategy_call(
        env,
        &pool_addr,
        &env.current_contract_address(),
        pool_action,
        charge_fee,
    );
    let mutation = PoolPositionMutation::from(&result);
    merge_debt_leg(
        env,
        account,
        action,
        &hub_debt,
        LegDirection::Entry {
            asset_decimals: mutation.asset_decimals,
        },
        &LegOutcome::from(&mutation),
        cache,
    );

    result.amount_received
}
