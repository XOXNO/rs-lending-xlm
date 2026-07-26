//! Strategy legs: wrappers over the borrow, withdraw, and repay position
//! primitives, moving tokens through the controller rather than the caller.
//!
//! No leg runs `require_auth` and none re-runs the post-pool health gate: the
//! calling strategy entrypoint owns both.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, DebtPosition, HubAssetKey, PoolNetSettleEntry, ScaledPositionRaw,
};
use soroban_sdk::{token, Address, Env, Vec};

use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::pool_net_settle_call;
use crate::payments as utils;
use crate::payments::balance_delta;
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawalRequest};
use crate::positions::{
    enforce_spoke_asset_flags, get_debt_position_or_panic, get_supply_position_or_panic, LegOutcome,
};

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
        counterparty: env.current_contract_address(),
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
    let debt_tok = token::Client::new(env, &req.debt.asset);

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
    let token = token::Client::new(env, &req.hub_asset.asset);
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
                    counterparty: destination.clone(),
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

/// Nets a supply leg against a debt leg on the identical `HubAssetKey` with
/// zero token transfer. Returns the real amount settled.
///
/// # Security Warning
/// * Performs no `require_auth` and re-runs no post-pool solvency gate: the
///   calling strategy entrypoint owns both.
pub(crate) fn net_settle_collateral_against_debt(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    hub_asset: &HubAssetKey,
    amount: i128,
    action: events::PositionAction,
) -> i128 {
    // Strategy chokepoint: paused blocks the settle, frozen still allows it,
    // matching the withdraw/repay primitives this replaces.
    enforce_spoke_asset_flags(env, cache, account.spoke_id, hub_asset, false);

    let supply_position = get_supply_position_or_panic(env, account, hub_asset);
    let debt_position = get_debt_position_or_panic(env, account, hub_asset);

    let pool_addr = cache.cached_pool_address();
    let entry = PoolNetSettleEntry {
        hub_asset: hub_asset.clone(),
        amount,
        supply_position: ScaledPositionRaw {
            scaled_amount: supply_position.scaled_amount.raw(),
        },
        debt_position: ScaledPositionRaw {
            scaled_amount: debt_position.scaled_amount.raw(),
        },
    };
    let result = pool_net_settle_call(env, &pool_addr, &entry);

    // Both sides settle through the same merge primitives the withdraw and
    // repay batch paths use, so spoke usage, the position maps, the index memo
    // and the event buffer stay in step across all three entry points.
    let supply_outcome = LegOutcome {
        new_scaled: Ray::from(result.supply_position.scaled_amount),
        market_index: result.market_index.clone(),
        amount: result.settled_amount,
    };
    // A delisted asset resolves to `Frozen`, matching the net-settle contract
    // that a settle never blocks on a listing that governance has since pulled.
    let refresh_spoke = withdraw::refresh_spoke_for_asset(cache, account, hub_asset);
    withdraw::merge_withdraw_leg(
        env,
        account,
        action,
        hub_asset,
        &refresh_spoke,
        &supply_outcome,
        cache,
    );

    let debt_outcome = LegOutcome {
        new_scaled: Ray::from(result.debt_position.scaled_amount),
        market_index: result.market_index,
        amount: result.settled_amount,
    };
    repay::merge_repay_leg(env, account, action, hub_asset, &debt_outcome, cache);

    result.settled_amount
}

/// Transfers any positive balance delta of `asset` back to `refund_to`.
fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let token = token::Client::new(env, asset);
    // D{asset.decimals}{Token(asset)} refund only the excess balance delta in same asset.
    let excess = balance_delta(env, &token, balance_before);
    if excess > 0 {
        token.transfer(&env.current_contract_address(), refund_to, &excess);
    }
}
