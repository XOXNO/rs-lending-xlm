//! Swap-debt strategy: borrows a new debt asset, swaps the borrowed amount
//! into the existing debt asset, and uses the proceeds to repay the
//! existing debt position on the same account.

use common::errors::GenericError;
use common::types::{HubAssetKey, StrategySwap};
use common::validation::require_positive_amount;
use soroban_sdk::{assert_with_error, vec, Address, Env};

use crate::account;
use crate::config;
use crate::context::Cache;
use crate::events::PositionAction;
use crate::positions::get_debt_position_or_panic;
use crate::strategies::{
    borrow_into_controller, prefetch_strategy_prices, repay_debt_from_controller, strategy_finalize,
    swap_tokens_or_passthrough, StrategyRepay,
};
use crate::{risk::validation, storage};

/// Parameters for [`process_swap_debt`]: the account, the existing debt
/// being replaced, the new debt asset and the amount of it to borrow, and
/// the swap route between them.
pub(crate) struct SwapDebtParams<'a> {
    pub account_id: u64,
    pub existing_debt: &'a HubAssetKey,
    pub new_debt_amount: i128,
    pub new_debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
}

/// Borrows `new_debt_amount` of `new_debt`, swaps the borrowed amount into
/// `existing_debt`'s asset, and repays the account's `existing_debt`
/// position with the swap proceeds, refunding any leftover to `caller`, then
/// re-runs post-transaction risk validation and finalizes the account.
///
/// Requires the caller's authorization and that the caller is the account
/// owner or an active protocol position manager listed among the account's
/// delegates. Panics if `existing_debt` and `new_debt` are the same asset, if
/// `existing_debt`'s hub is inactive, if `new_debt_amount` is not positive, or
/// if the account has no open position in `existing_debt`.
pub(crate) fn process_swap_debt(env: &Env, caller: &Address, params: SwapDebtParams<'_>) {
    let SwapDebtParams {
        account_id,
        existing_debt,
        new_debt_amount,
        new_debt,
        swap,
    } = params;

    caller.require_auth();
    validation::require_not_flash_loaning(env);

    assert_with_error!(
        env,
        existing_debt != new_debt,
        GenericError::AssetsAreTheSame
    );
    config::require_hub_active(env, existing_debt.hub_id);
    require_positive_amount(env, new_debt_amount);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);
    let existing_pos = get_debt_position_or_panic(env, &account, existing_debt);

    let extra_assets = vec![env, existing_debt.asset.clone(), new_debt.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    let amount_received =
        borrow_into_controller(env, &mut account, new_debt, new_debt_amount, true, PositionAction::SwDebtR, &mut cache);

    let repay_amount = swap_tokens_or_passthrough(
        env,
        caller,
        &new_debt.asset,
        amount_received,
        &existing_debt.asset,
        swap,
    );

    repay_debt_from_controller(
        env,
        &mut account,
        &mut cache,
        caller,
        StrategyRepay {
            debt: existing_debt,
            debt_available: repay_amount,
            debt_pos: &existing_pos,
            action: PositionAction::SwDebtR,
        },
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}
