//! Strategy wrappers for borrow, withdraw, and repay primitives.

use common::errors::GenericError;
use common::types::{Account, AccountPosition, DebtPosition, HubAssetKey};
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::payments::{self as utils, EventContext};
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawalRequest, WITHDRAW_ALL_SENTINEL};
use crate::strategies::swap::balance_delta;

pub(crate) struct StrategyRepay<'a> {
    pub debt: &'a HubAssetKey,
    pub debt_available: i128,
    pub debt_pos: &'a DebtPosition,
    pub action: events::PositionAction,
}

pub(crate) struct StrategyWithdraw<'a> {
    pub hub_asset: &'a HubAssetKey,
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

pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address();
    let debt_tok = soroban_sdk::token::Client::new(env, &req.debt.asset);

    // D{debt_token.decimals}{Token(debt_token)} repay transfer and debt request use same token units.
    utils::transfer_amount(
        env,
        &req.debt.asset,
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
            hub_asset: req.debt,
            position: req.debt_pos,
            amount: req.debt_available,
        },
        cache,
    );

    refund_controller_balance_delta(
        env,
        &req.debt.asset,
        controller_balance_before_repay,
        caller,
    );
}

pub(crate) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let token = soroban_sdk::token::Client::new(env, &req.hub_asset.asset);
    // D{asset.decimals}{Token(asset)} withdrawal result is measured from live balance delta.
    let balance_before = token.balance(&env.current_contract_address());

    withdraw::execute_withdrawal(
        env,
        account,
        controller_event_context(env, req.action),
        WithdrawalRequest {
            hub_asset: req.hub_asset,
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
    let deposit_keys: Vec<HubAssetKey> = account.supply_positions.keys();
    for hub_asset in deposit_keys.iter() {
        if let Some(pos) = account.supply_positions.get(hub_asset.clone()) {
            let pos: AccountPosition = (&pos).into();
            withdraw::execute_withdrawal(
                env,
                account,
                EventContext {
                    caller: destination.clone(),
                    action: events::PositionAction::CloseWd,
                },
                WithdrawalRequest {
                    hub_asset: &hub_asset,
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
