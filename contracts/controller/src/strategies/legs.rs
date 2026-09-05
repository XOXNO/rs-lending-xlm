use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, DebtPosition, HubAssetKey, PoolNetSettleEntry, ScaledPositionRaw,
    StrategySwap,
};
use soroban_sdk::{token, Address, Env, Vec};

use crate::constants::WITHDRAW_ALL_SENTINEL;
use crate::context::Context;
use crate::events;
use crate::external::pool::pool_net_settle_call;
use crate::payments::{self, balance_delta_since, refund_controller_balance_delta};
use crate::positions::{
    enforce_spoke_asset_flags, get_debt_position_or_panic, get_supply_position_or_panic,
    merge_debt_leg, merge_withdraw_leg, FreezePolicy, LegDirection, LegOutcome, WithdrawKind,
};
use crate::positions::{execute_withdrawal, WithdrawalRequest};
use crate::positions::{repay_prefunded_position, RepaymentRequest};
use crate::storage;
use crate::strategies::swap::swap_tokens_or_passthrough;

pub(crate) struct StrategyRepay<'a> {
    pub debt: &'a HubAssetKey,
    /// Controller funds available to transfer; the pool's receipt may differ.
    pub debt_available: i128,
    pub debt_pos: &'a DebtPosition,
    pub action: events::PositionAction,
}

pub(super) struct StrategyWithdraw<'a> {
    pub hub_asset: &'a HubAssetKey,
    pub amount: i128,
    pub position: &'a AccountPosition,
    pub action: events::PositionAction,
}

/// Funds repayment from controller custody using the pool's measured receipt.
/// Refunds only the balance increase from the repayment call to `caller`.
pub(crate) fn repay_debt_from_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
    caller: &Address,
    req: StrategyRepay<'_>,
) {
    let debt_pool_addr = cache.cached_pool_address();
    let debt_tok = token::Client::new(env, &req.debt.asset);

    let received = payments::transfer_amount_measured(
        env,
        &req.debt.asset,
        &env.current_contract_address(),
        &debt_pool_addr,
        req.debt_available,
        GenericError::InternalError,
    );

    // Snapshot after funding so only repayment refunds go to the caller.
    let controller_balance_before_repay = debt_tok.balance(&env.current_contract_address());

    repay_prefunded_position(
        env,
        account,
        &env.current_contract_address(),
        req.action,
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

/// Withdraws supply into controller custody and returns its measured receipt.
pub(super) fn withdraw_collateral_to_controller(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
    req: StrategyWithdraw<'_>,
) -> i128 {
    let controller = env.current_contract_address();
    let balance_before = token::Client::new(env, &req.hub_asset.asset).balance(&controller);

    storage::with_flash_guard(env, || {
        execute_withdrawal(
            env,
            account,
            &controller,
            req.action,
            WithdrawalRequest {
                hub_asset: req.hub_asset,
                amount: req.amount,
                position: req.position,
            },
            cache,
        );
    });

    balance_delta_since(env, &req.hub_asset.asset, &controller, balance_before)
}

/// Closes every supply position to `destination` with the `CloseWd` action.
pub(crate) fn execute_withdraw_all(
    env: &Env,
    account: &mut Account,
    destination: &Address,
    cache: &mut Context,
) {
    let deposit_keys: Vec<HubAssetKey> = account.supply_positions.keys();
    for hub_asset in deposit_keys.iter() {
        if let Some(pos) = account.supply_positions.get(hub_asset.clone()) {
            let pos: AccountPosition = (&pos).into();
            execute_withdrawal(
                env,
                account,
                destination,
                events::PositionAction::CloseWd,
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

/// Nets same-market supply against debt without token transfers.
/// Updates both legs from the pool result and returns the settled amount.
pub(crate) fn net_settle_collateral_against_debt(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
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

    merge_withdraw_leg(
        env,
        account,
        action,
        hub_asset,
        WithdrawKind::Normal,
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

/// Withdraws to the controller, then swaps its measured receipt into `token_out`.
/// Matching assets pass through unchanged; returns the available output.
pub(crate) fn withdraw_and_swap_from_supply(
    env: &Env,
    account: &mut Account,
    cache: &mut Context,
    caller: &Address,
    from: &HubAssetKey,
    amount: i128,
    token_out: &Address,
    swap: &StrategySwap,
    action: events::PositionAction,
) -> i128 {
    let supply_pos = get_supply_position_or_panic(env, account, from);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        account,
        cache,
        StrategyWithdraw {
            hub_asset: from,
            amount,
            position: &supply_pos,
            action,
        },
    );

    swap_tokens_or_passthrough(env, caller, &from.asset, actual_withdrawn, token_out, swap)
}
