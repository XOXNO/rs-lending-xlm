//! Debt refinance: borrow new debt → aggregator swap → repay existing.
//!
//! Owner/delegate auth. Intermediate borrow skips HF; `strategy_finalize`
//! re-checks LTV/HF.

use common::errors::GenericError;
use common::types::{HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, vec, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::account;
use crate::context::Cache;
use crate::events;
use crate::positions::get_debt_position_or_panic;
use crate::strategies::{
    borrow_for_strategy, prefetch_strategy_prices, repay_debt_from_controller, strategy_finalize,
    swap_tokens_or_passthrough, StrategyRepay,
};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

pub(crate) struct SwapDebtParams<'a> {
    pub account_id: u64,
    pub existing_debt: &'a HubAssetKey,
    pub new_debt_amount: i128,
    pub new_debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
}

#[contractimpl]
impl Controller {
    /// Refinances `amount` of `existing_debt` into `new_debt` via aggregator route.
    /// Owner or active delegate. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AssetsAreTheSame` — identical `(hub, asset)` pair.
    /// * `AmountMustBePositive` / `HubNotActive` — preflight.
    /// * `NotAuthorized` — caller is neither owner nor active delegate.
    /// * `DebtPositionNotFound` — no debt position for `existing_debt`.
    /// * Borrow/swap/repay errors from the nested legs.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    #[when_not_paused]
    pub fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    ) {
        process_swap_debt(
            &env,
            &caller,
            SwapDebtParams {
                account_id,
                existing_debt: &existing_debt,
                new_debt_amount: amount,
                new_debt: &new_debt,
                swap: &swap,
            },
        );
    }
}

/// Refinance: borrow new debt → swap to existing debt token → repay existing.
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

    // Reject identical (hub, asset); same token across hubs is passthrough.
    assert_with_error!(
        env,
        existing_debt != new_debt,
        GenericError::AssetsAreTheSame
    );
    validation::require_hub_active(env, existing_debt.hub_id);
    validation::require_positive_amount(env, new_debt_amount);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);
    let existing_pos = get_debt_position_or_panic(env, &account, existing_debt);

    let extra_assets = vec![env, existing_debt.asset.clone(), new_debt.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    let amount_received =
        borrow_for_strategy(env, &mut account, new_debt, new_debt_amount, &mut cache);

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
            action: events::PositionAction::SwDebtR,
        },
    );

    strategy_finalize(env, account_id, &mut account, &mut cache);
}
