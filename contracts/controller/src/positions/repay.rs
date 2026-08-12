
use common::errors::GenericError;
use common::types::{Account, DebtPosition, HubAssetKey, PoolAction, PoolPositionMutation};
use soroban_sdk::{vec, Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::pool_repay_call;
use crate::payments;
use crate::positions::{
    enforce_spoke_asset_flags, finalize_position_flow, for_each_leg, get_debt_position_or_panic,
    make_pool_action, merge_debt_leg, require_position_caller, AggregatedPayments, FreezePolicy,
    HubPayment, LegDirection, LegOutcome, PositionSides,
};
use crate::storage;
use common::validation::expect_invariant;

pub(crate) struct RepaymentRequest<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub position: &'a DebtPosition,
    pub amount: i128,
}

pub(crate) fn process_repay(
    env: &Env,
    caller: &Address,
    account_id: u64,
    payments: &Vec<HubPayment>,
) {
    require_position_caller(env, caller);

    let aggregated = payments::aggregate_positive_payments(env, payments);

    let mut account = storage::get_account_borrow_only(env, account_id);
    let mut cache = Cache::new(env);

    settle_repay(env, caller, &mut account, &aggregated, &mut cache);

    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::DEBT,
        false,
    );
}

fn settle_repay(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) {
    let actions = build_repay_actions(env, caller, account, aggregated, cache);
    apply_repay_batch(
        env,
        account,
        caller,
        events::PositionAction::Repay,
        &actions,
        cache,
    );
}

fn build_repay_actions(
    env: &Env,
    caller: &Address,
    account: &Account,
    aggregated: &AggregatedPayments,
    cache: &mut Cache,
) -> Vec<PoolAction> {
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
            caller,
            &pool_addr,
            amount,
            GenericError::AmountMustBePositive,
        );
        actions.push_back(make_pool_action(&position, amount_in, hub_asset.clone()));
    }
    actions
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
    let actions = vec![
        env,
        make_pool_action(req.position, req.amount, req.hub_asset.clone()),
    ];
    let results = apply_repay_batch(env, account, &counterparty, action, &actions, cache);
    expect_invariant(env, results.get(0))
}
