//! Borrow-flow logic for the controller: aggregates requested borrow legs, calls the pool
//! to open or increase debt positions, applies the pool results to account debt state, and
//! enforces solvency afterward. Also provides the borrow entry points used by strategy
//! execution (leveraged multiply positions) and position migration.

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

/// Executes a borrow of `borrows` against hub debt positions for `account_id`, crediting the
/// borrowed funds to `to` (or `caller` if `to` is `None`).
///
/// Requires `caller`'s authorization and reverts if a flash loan is in progress, and requires
/// `caller` to be the account owner or an active protocol position manager listed among the
/// account's delegates. Aggregates `borrows` into per-hub-asset amounts, validates entry gates
/// for each asset, and calls the pool to create or increase the corresponding debt positions.
/// Re-runs solvency checks after the pool call and persists the debt position map, plus the
/// supply position map if LTV restamping touched it.
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

/// Builds pool borrow entries for `aggregated` and applies them against the pool, updating
/// `account`'s debt positions from the results.
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

/// Builds one [`PoolBorrowEntry`] per hub asset in `aggregated`, using each asset's existing or
/// newly created debt position.
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

/// Calls the pool to execute `entries` as a borrow batch crediting `recipient`, then merges each
/// leg's result into `account`'s debt positions.
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

/// Borrows `amount` of `hub_debt` into `account` via the pool strategy-borrow path with the
/// controller contract as recipient. When `charge_fee` is set the pool applies its strategy fee.
/// `action` selects the event tag (`Multiply` or `Migrate`). Returns the amount actually received.
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
