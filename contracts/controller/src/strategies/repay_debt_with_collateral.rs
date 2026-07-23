//! Repay debt from collateral: withdraw → (swap) → repay; optional full close.
//!
//! Owner/delegate auth. Same hub-asset nets in-pool; distinct assets swap via
//! aggregator. `strategy_finalize` re-checks LTV/HF.

use common::errors::{CollateralError, GenericError};
use common::types::{Account, HubAssetKey, StrategySwap};
use soroban_sdk::{assert_with_error, contractimpl, vec, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::account;
use crate::context::Cache;
use crate::events;
use crate::positions::{get_debt_position_or_panic, get_supply_position_or_panic};
use crate::strategies::{
    execute_withdraw_all, net_settle_collateral_against_debt, prefetch_strategy_prices,
    repay_debt_from_controller, strategy_finalize, swap_tokens_or_passthrough,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

pub(crate) struct RepayWithCollateralParams<'a> {
    pub account_id: u64,
    pub collateral: &'a HubAssetKey,
    pub collateral_amount: i128,
    pub debt: &'a HubAssetKey,
    pub swap: &'a StrategySwap,
    pub close_position: bool,
}

#[contractimpl]
impl Controller {
    /// Repays `debt` using `collateral_amount` of `collateral` (swap when distinct).
    /// Owner or active delegate. `close_position` fully exits remaining collateral
    /// only when debt is already zero. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` / `HubNotActive` — preflight.
    /// * `NotAuthorized` — caller is neither owner nor active delegate.
    /// * `CollateralPositionNotFound` / `DebtPositionNotFound` — missing legs.
    /// * `CannotCloseWithRemainingDebt` — `close_position` while debt remains.
    /// * `InvalidPayments` — non-empty swap on same-asset net path.
    /// * Swap/withdraw/repay errors from the nested legs.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
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

/// Withdraw collateral → (swap to debt token) → repay; optional full close.
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

    validation::require_positive_amount(env, collateral_amount);
    validation::require_hub_active(env, collateral.hub_id);
    validation::require_hub_active(env, debt.hub_id);

    let mut account = storage::get_account(env, account_id);
    account::require_owner_or_delegate(env, account_id, caller, &account.owner);
    let mut cache = Cache::new(env);

    let extra_assets = vec![env, collateral.asset.clone(), debt.asset.clone()];
    prefetch_strategy_prices(&mut cache, &account, &extra_assets);

    // Same hub-asset: in-pool net; else withdraw → swap → repay.
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

    // Full collateral exit only if debt is already zero.
    close_remaining_collateral_if_requested(env, &mut account, caller, &mut cache, close_position);

    strategy_finalize(env, account_id, &mut account, &mut cache);
}

/// Same (hub, asset): net collat against debt in-pool (no token round-trip).
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

/// Distinct assets: withdraw collat → swap to debt token → repay from controller.
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
    let collateral_pos = get_supply_position_or_panic(env, account, collateral);
    let debt_pos = get_debt_position_or_panic(env, account, debt);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        account,
        cache,
        StrategyWithdraw {
            hub_asset: collateral,
            amount: collateral_amount,
            position: &collateral_pos,
            action: events::PositionAction::RpColWd,
        },
    );

    let debt_available = swap_tokens_or_passthrough(
        env,
        caller,
        &collateral.asset,
        actual_withdrawn,
        &debt.asset,
        swap,
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

/// Full collateral exit; requires zero remaining debt.
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
