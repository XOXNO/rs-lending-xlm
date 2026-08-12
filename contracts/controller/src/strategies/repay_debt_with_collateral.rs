//! Repay-with-collateral strategy: reduces a debt position using collateral
//! from the same account, either by netting a matching asset directly against
//! the debt or by withdrawing collateral, swapping it into the debt asset,
//! and repaying with the proceeds. Optionally closes out all remaining
//! collateral once the account is debt-free.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, HubAssetKey, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, vec, Address, Env};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events;
use crate::positions::get_debt_position_or_panic;
use crate::strategies::{
    execute_withdraw_all, net_settle_collateral_against_debt, prefetch_strategy_prices,
    repay_debt_from_controller, strategy_finalize, withdraw_and_swap_from_supply, StrategyRepay,
};
use crate::{risk::validation, storage};

/// Inputs to [`process_repay_debt_with_collateral`]: the target account, the
/// collateral asset and amount to draw down, the debt asset to repay, the
/// swap route from collateral to debt, and whether to withdraw all remaining
/// collateral once the account has no debt left.
pub(crate) struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

/// Repays part or all of an account's debt using its own collateral.
/// Requires `caller`'s authorization and that `caller` is the account owner or
/// an authorized position-manager delegate; rejects the call while a flash loan
/// is in progress; requires a positive `collateral_amount` and both hubs active.
/// Prefetches prices, then either nets the collateral directly against matching
/// debt (when `collateral == debt`) or withdraws and swaps the collateral into
/// the debt asset before repaying. If `close_position` is set, withdraws all
/// remaining collateral once no debt remains. Finalizes the account.
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

    caller.require_auth();
    validation::require_not_flash_loaning(env);

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

/// Nets `amount` of `hub_asset` from the account's supply position directly
/// against its debt position of the same asset. Requires `swap` to be empty,
/// panicking with `InvalidPayments` otherwise.
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

/// Withdraws `collateral_amount` of `collateral` from supply, swaps it into
/// the debt asset per `swap`, and repays that amount onto the account's debt
/// position for `debt`.
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

/// Does nothing unless `close_position` is set. When set, panics with
/// `CollateralError::CannotCloseWithRemainingDebt` if the account still has
/// any borrow positions, otherwise withdraws all remaining supply positions
/// to `caller`.
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
