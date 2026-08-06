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

pub(crate) struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

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
