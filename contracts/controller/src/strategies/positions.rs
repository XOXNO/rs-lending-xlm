//! Strategy wrappers over position primitives (`borrow`, `withdraw`, `repay`).
//!
//! These helpers mutate in-memory account state and record cache events; callers
//! must reach `strategy_finalize` before the transaction ends when debt is opened.

use common::errors::GenericError;
use controller_interface::types::{Account, AccountPosition, DebtPosition};
use soroban_sdk::{Address, Env, Vec};

use crate::cache::Cache;
use crate::events;
use crate::helpers::utils::{self, EventContext};
use crate::positions::borrow;
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawalRequest, WITHDRAW_ALL_SENTINEL};
use crate::strategies::swap::balance_delta;

pub(crate) struct StrategyRepay<'a> {
    pub debt_token: &'a Address,
    pub debt_available: i128,
    pub debt_pos: &'a DebtPosition,
    pub action: events::PositionAction,
}

pub(crate) struct StrategyWithdraw<'a> {
    pub asset: &'a Address,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub action: events::PositionAction,
}

fn controller_event_context(env: &Env, action: events::PositionAction) -> EventContext {
    EventContext {
        caller: env.current_contract_address(),
        action,
    }
}

pub(crate) fn open_strategy_borrow(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
    asset: &Address,
    amount: i128,
) -> i128 {
    // D{asset.decimals}{Token(asset)} debt opens for `amount`; return is net balance after Token(asset) fee.
    borrow::borrow_for_strategy(env, account, asset, amount, cache)
}

/// Zero-fee strategy borrow for Blend migration; returns the amount received.
pub(crate) fn open_migration_borrow(
    env: &Env,
    cache: &mut Cache,
    account: &mut Account,
    asset: &Address,
    amount: i128,
) -> i128 {
    // D{asset.decimals}{Token(asset)} zero-fee migration borrow; debt amount equals received token delta.
    borrow::borrow_for_migration(env, account, asset, amount, cache)
}

pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address();
    let debt_tok = soroban_sdk::token::Client::new(env, req.debt_token);

    // D{debt_token.decimals}{Token(debt_token)} repay transfer and debt request use same token units.
    utils::transfer_amount(
        env,
        req.debt_token,
        &env.current_contract_address(),
        &debt_pool_addr,
        req.debt_available,
        GenericError::InternalError,
    );

    // D{debt_token.decimals}{Token(debt_token)} post-repay positive delta is excess refund.
    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    repay::execute_repayment(
        env,
        account,
        controller_event_context(env, req.action),
        RepaymentRequest {
            asset: req.debt_token,
            position: req.debt_pos,
            amount: req.debt_available,
        },
        cache,
    );

    refund_controller_balance_delta(env, req.debt_token, controller_balance_before_repay, caller);
}

pub(crate) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let token = soroban_sdk::token::Client::new(env, req.asset);
    // D{asset.decimals}{Token(asset)} withdrawal result is measured from live balance delta.
    let balance_before = token.balance(&env.current_contract_address());

    withdraw::execute_withdrawal(
        env,
        account,
        controller_event_context(env, req.action),
        WithdrawalRequest {
            asset: req.asset,
            amount: req.amount,
            position: req.position,
        },
        cache,
    );

    balance_delta(env, &token, balance_before)
}

pub(crate) fn execute_withdraw_all(
    env: &Env,
    account: &mut Account,
    destination: &Address,
    cache: &mut Cache,
) {
    let deposit_keys: Vec<Address> = account.supply_positions.keys();
    for asset in deposit_keys.iter() {
        if let Some(pos) = account.supply_positions.get(asset.clone()) {
            let pos: AccountPosition = (&pos).into();
            withdraw::execute_withdrawal(
                env,
                account,
                EventContext {
                    caller: destination.clone(),
                    action: events::PositionAction::CloseWd,
                },
                WithdrawalRequest {
                    asset: &asset,
                    amount: WITHDRAW_ALL_SENTINEL,
                    position: &pos,
                },
                cache,
            );
        }
    }
}

fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let token = soroban_sdk::token::Client::new(env, asset);
    // D{asset.decimals}{Token(asset)} refund only the excess balance delta in same asset.
    let excess = balance_delta(env, &token, balance_before);
    if excess > 0 {
        token.transfer(&env.current_contract_address(), refund_to, &excess);
    }
}
