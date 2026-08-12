//! Reusable position "legs" shared by higher-level strategies: repaying debt
//! and withdrawing collateral through the controller's own balance, closing
//! out all supply positions, and netting a supply position directly against a
//! debt position via the pool's net-settle call.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, DebtPosition, HubAssetKey, PoolNetSettleEntry, ScaledPositionRaw,
};
use soroban_sdk::{panic_with_error, token, Address, Env, Vec};

use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Cache;
use crate::events;
use crate::events::EventContext;
use crate::external::pool::pool_net_settle_call;
use crate::payments;
use crate::positions::repay::{self, RepaymentRequest};
use crate::positions::withdraw::{self, WithdrawalRequest};
use crate::positions::{
    enforce_spoke_asset_flags, get_debt_position_or_panic, get_supply_position_or_panic,
    merge_debt_leg, FreezePolicy, LegDirection, LegOutcome,
};

/// Parameters for [`repay_debt_from_controller`]: the debt to repay, the
/// amount available for repayment, the debt position being reduced, and the
/// event action to record.
pub(crate) struct StrategyRepay<'a> {
    pub debt: &'a HubAssetKey,
    pub debt_available: i128,
    pub debt_pos: &'a DebtPosition,
    pub action: events::PositionAction,
}

/// Parameters for [`withdraw_collateral_to_controller`]: the asset and amount
/// to withdraw, the supply position being reduced, and the event action to
/// record.
pub(crate) struct StrategyWithdraw<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub action: events::PositionAction,
}

/// Builds an `EventContext` whose counterparty is the controller contract
/// itself, for legs that route funds through the controller's own balance.
fn controller_event_context(env: &Env, action: events::PositionAction) -> EventContext {
    EventContext {
        counterparty: env.current_contract_address(),
        action,
    }
}

/// Transfers `req.debt_available` of the debt asset from the controller to
/// the pool, measures the amount the pool actually received, and repays the
/// account's debt position by that received amount. Any leftover controller
/// balance of the debt asset created by the repayment is refunded to
/// `caller`.
pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address();
    let debt_tok = token::Client::new(env, &req.debt.asset);

    // Measure pool receipt so strategy debt burn matches tokens that arrived.
    let received = payments::transfer_amount_measured(
        env,
        &req.debt.asset,
        &env.current_contract_address(),
        &debt_pool_addr,
        req.debt_available,
        GenericError::InternalError,
    );

    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    repay::execute_repayment(
        env,
        account,
        controller_event_context(env, req.action),
        RepaymentRequest {
            hub_asset: req.debt,
            position: req.debt_pos,
            amount: received,
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

/// Withdraws `req.amount` of `req.hub_asset` from the account's supply
/// position into the controller's own balance and returns the resulting
/// increase in the controller's token balance.
pub(crate) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let token = token::Client::new(env, &req.hub_asset.asset);

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

    token.balance(&env.current_contract_address()).checked_sub(balance_before).unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

/// Withdraws every supply position held by `account` in full, sending each
/// asset to `destination` and recording each as a `CloseWd` position action.
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

/// Nets `amount` of `hub_asset` between the account's supply and debt
/// positions in a single pool call, enforcing spoke freeze policy for the
/// exiting collateral first. Applies the pool's settlement result to both the
/// supply leg (via a spoke refresh) and the debt leg, then returns the amount
/// actually settled.
pub(crate) fn net_settle_collateral_against_debt(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    hub_asset: &HubAssetKey,
    amount: i128,
    action: events::PositionAction,
) -> i128 {
    enforce_spoke_asset_flags(
        env,
        cache,
        account.spoke_id,
        hub_asset,
        FreezePolicy::AllowOnExit,
    );

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

    let supply_outcome = LegOutcome {
        new_scaled: Ray::from(result.supply_position.scaled_amount),
        market_index: result.market_index.clone(),
        amount: result.settled_amount,
    };

    let refresh_spoke = withdraw::spoke_refresh_for_leg(
        withdraw::WithdrawKind::Normal,
        cache,
        account,
        hub_asset,
        supply_outcome.new_scaled,
    );
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
    merge_debt_leg(
        env,
        account,
        action,
        hub_asset,
        LegDirection::Exit,
        &debt_outcome,
        cache,
    );

    result.settled_amount
}

/// Transfers any increase in the controller's balance of `asset` since
/// `balance_before` to `refund_to`. Does nothing if the balance has not
/// increased.
fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let token = token::Client::new(env, asset);

    let excess = token.balance(&env.current_contract_address()).checked_sub(balance_before).unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    if excess > 0 {
        token.transfer(&env.current_contract_address(), refund_to, &excess);
    }
}
