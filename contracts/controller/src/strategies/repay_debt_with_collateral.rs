//! Repays debt by withdrawing and swapping collateral.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, AccountPosition, DebtPosition, HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::events;
use crate::strategies::{
    execute_withdraw_all, prefetch_strategy_oracles, repay_debt_from_controller, strategy_finalize,
    swap_tokens, withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

pub struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral: HubAssetKey,
        collateral_amount: i128,
        debt: HubAssetKey,
        swap: Bytes,
        close_position: bool,
    ) {
        process_repay_debt_with_collateral(
            &env,
            &caller,
            RepayWithCollateralParams {
                account_id,
                collateral: &collateral,
                collateral_amount,
                debt: &debt,
                swap: &swap,
                close_position,
            },
        );
    }
}

pub fn process_repay_debt_with_collateral(
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
    validation::require_positive_amount(env, collateral_amount);

    // The withdraw and repay legs settle on their own hubs; neither path gates
    // hub membership, so assert it here.
    validation::require_hub_active(env, collateral.hub_id);
    validation::require_hub_active(env, debt.hub_id);

    let mut account = storage::get_account(env, account_id);
    crate::account::require_owner_or_delegate(env, account_id, caller, &account.owner);

    let mut cache = Cache::new(env);

    let (collateral_pos, debt_pos) =
        load_repay_with_collateral_positions(env, &account, collateral, debt);

    let extra_assets = soroban_sdk::vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_oracles(&mut cache, &account, &extra_assets);

    // D{collateral_token.decimals}{Token(collateral_token)} requested withdrawal to live balance delta.
    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        &mut account,
        &mut cache,
        StrategyWithdraw {
            hub_asset: collateral,
            amount: collateral_amount,
            position: &collateral_pos,
            action: events::PositionAction::RpColWd,
        },
    );

    // D{collateral_token.decimals}{Token(collateral_token)} -> Token(debt_token), unless same asset.
    let debt_available =
        swap_or_net_collateral_to_debt(env, caller, collateral, debt, actual_withdrawn, swap);
    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        StrategyRepay {
            debt,
            debt_available,
            debt_pos: &debt_pos,
            action: events::PositionAction::RpColR,
        },
    );

    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

fn load_repay_with_collateral_positions(
    env: &Env,
    account: &Account,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
) -> (AccountPosition, DebtPosition) {
    let collateral_pos = account
        .supply_positions
        .get(collateral.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::CollateralPositionNotFound));
    let debt_pos = account
        .borrow_positions
        .get(debt.clone())
        .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

    ((&collateral_pos).into(), (&debt_pos).into())
}

fn swap_or_net_collateral_to_debt(
    env: &Env,
    caller: &Address,
    collateral: &HubAssetKey,
    debt: &HubAssetKey,
    collateral_amount: i128,
    swap: &StrategySwap,
) -> i128 {
    if collateral.asset == debt.asset {
        // Same-asset netting never consults `swap`; require an empty payload so a
        // caller-supplied route is not silently dropped.
        assert_with_error!(env, swap.is_empty(), GenericError::InvalidPayments);
        return collateral_amount;
    }

    swap_tokens(
        env,
        caller,
        &collateral.asset,
        collateral_amount,
        &debt.asset,
        swap,
    )
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
