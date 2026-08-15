use common::errors::{CollateralError, GenericError};
use common::types::{Account, HubAssetKey, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, vec, Address, Env};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events;
use crate::positions::get_debt_position_or_panic;
use crate::storage;
use crate::strategies::{
    execute_withdraw_all, net_settle_collateral_against_debt, prefetch_strategy_prices,
    repay_debt_from_controller, strategy_finalize, withdraw_and_swap_from_supply, StrategyRepay,
};

pub(crate) struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

/// Repays `debt` using `collateral` for `caller`'s account: nets supply
/// directly against debt on the pool when they are the same market,
/// otherwise withdraws `collateral_amount` of collateral, swaps it into the
/// debt asset, and repays with the proceeds. When `close_position` is set
/// and no debt remains afterward, also withdraws all remaining collateral to
/// `caller` before the standard solvency finalize.
pub(crate) fn process_repay_debt_with_collateral(
    env: &Env,
    caller: &Address,
    params: RepayWithCollateralParams<'_>,
) {
    let RepayWithCollateralParams {
        account_id,
        collateral,
        collateral_amount,
        debt,
        swap,
        close_position,
    } = params;

    crate::strategies::require_strategy_caller(env, caller);

    require_positive_amount(env, collateral_amount);
    config::require_hub_active(env, collateral.hub_id);
    config::require_hub_active(env, debt.hub_id);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);

    let extra_assets = vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    if collateral == debt {
        repay_same_asset_net(
            env,
            &mut account,
            &mut cache,
            collateral,
            collateral_amount,
            swap,
        );
    } else {
        repay_via_collateral_swap(
            env,
            caller,
            &mut account,
            &mut cache,
            collateral,
            collateral_amount,
            debt,
            swap,
        );
    }

    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

/// Nets `amount` of `hub_asset` supply directly against its debt on the pool
/// without moving tokens. Panics with `InvalidPayments` if `swap` is
/// non-empty, since no conversion is needed for a same-asset repay.
fn repay_same_asset_net(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    hub_asset: &HubAssetKey,
    amount: i128,
    swap: &StrategySwap,
) {
    assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
    net_settle_collateral_against_debt(
        env,
        account,
        cache,
        hub_asset,
        amount,
        events::PositionAction::RpColNet,
    );
}

/// Withdraws `collateral_amount` of `collateral` to the controller, swaps it
/// into `debt`'s asset, and repays that amount against `account`'s existing
/// debt position. Requires `account` to already hold a debt position in
/// `debt`, checked before any collateral is withdrawn.
fn repay_via_collateral_swap(
    env: &Env,
    caller: &Address,
    account: &mut Account,
    cache: &mut Cache,
    collateral: &HubAssetKey,
    collateral_amount: i128,
    debt: &HubAssetKey,
    swap: &StrategySwap,
) {
    // Fail fast if debt is missing before withdrawing collateral.
    let debt_pos = get_debt_position_or_panic(env, account, debt);

    let debt_available = withdraw_and_swap_from_supply(
        env,
        account,
        cache,
        caller,
        collateral,
        collateral_amount,
        &debt.asset,
        swap,
        events::PositionAction::RpColWd,
    );

    repay_debt_from_controller(
        env,
        account,
        cache,
        caller,
        StrategyRepay {
            debt,
            debt_available,
            debt_pos: &debt_pos,
            action: events::PositionAction::RpColR,
        },
    );
}

/// Withdraws all of `account`'s remaining supply positions to `caller` when
/// `close_position` is set; no-op otherwise. Panics with
/// `CannotCloseWithRemainingDebt` if any debt remains.
fn close_remaining_collateral_if_requested(
    env: &Env,
    account: &mut Account,
    caller: &Address,
    cache: &mut Cache,
    close_position: bool,
) {
    if !close_position {
        return;
    }

    assert_with_error!(
        env,
        account.borrow_positions.is_empty(),
        CollateralError::CannotCloseWithRemainingDebt
    );

    execute_withdraw_all(env, account, caller, cache);
}
