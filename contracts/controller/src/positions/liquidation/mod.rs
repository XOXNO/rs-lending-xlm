//! Liquidation and residual bad-debt socialization.
//!
//! Pipeline: plan (HF < 1, price, normalize) → apply (repay then seize) →
//! optional bad-debt cleanup. Permissionless keepers; not gated by global pause.
//! Spoke pause blocks inbound repay tokens; paused collateral remains seizable.
//! See `architecture/INVARIANTS.md` §3.3.

use crate::risk;
mod apply;
mod bad_debt;
pub(crate) mod math;
mod plan;

pub(crate) use plan::execute_liquidation;

use common::errors::CollateralError;
use common::types::{Account, HubAssetKey};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Vec};

use self::math::is_socializable_bad_debt;
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::payments;
use crate::positions::{persist_account_positions, AggregatedPayments, HubPayment, PositionSides};
use crate::risk::validation;
use crate::storage;
use crate::{Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Liquidates an underwater account: liquidator pays selected debt and
    /// receives bonused collateral. Permissionless; liquidator auth; not the
    /// owner. Requires HF < 1. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `InvalidPayments` — empty debt payment list or empty post-normalization set.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `SelfLiquidationNotAllowed` — `liquidator` is the account owner.
    /// * `SpokeAssetPaused` — a repaid debt leg's listing is paused.
    /// * `HealthFactorTooHigh` — account HF is still at or above one.
    /// * `OracleNotConfigured` / `PoolNotInitialized` — fail-closed pricing path.
    ///
    /// # Events
    /// * topics — `["position", "liquidation"]`
    /// * topics — `["position", "batch_update"]`
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    /// Socializes residual bad debt into the pool (no liquidator proceeds).
    /// Permissionless; caller auth for accountability. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `DebtPositionNotFound` — the account carries no debt.
    /// * `CannotCleanBadDebt` — not eligible socializable residual.
    ///
    /// # Events
    /// * topics — `["debt", "bad_debt"]`
    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        caller.require_auth();
        validation::require_not_flash_loaning(&env);
        clean_bad_debt_standalone(&env, account_id);
    }
}

/// Auth, plan, transfer repay + seize, persist both sides, then residual bad debt.
///
/// Does not use `finalize_position_flow`: persists BOTH maps without
/// `remove_if_empty`, then may delete the account via bad-debt cleanup.
pub(crate) fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);
    let aggregated = payments::aggregate_positive_payments(env, debt_payments);

    let mut cache = Cache::new(env);

    validate_liquidation_inputs(env, &account, liquidator, &aggregated);

    let liquidation_plan = plan::build_liquidation_plan(env, &account, &aggregated, &mut cache);
    // Only `result.repaid` transfers; refunds are informational.
    let result = liquidation_plan.into_result();

    validation::require_non_empty_payments(env, &result.repaid);

    apply::apply_liquidation_repayments(env, liquidator, &mut account, &result.repaid, &mut cache);
    apply::apply_liquidation_seizures(env, liquidator, &mut account, &result.seized, &mut cache);

    LiquidationEvent {
        liquidator: liquidator.clone(),
        account_id,
        repaid_usd_wad: result.max_debt_usd,
        bonus_bps: result.bonus_bps,
    }
    .publish(env);

    let post_totals = risk::calculate_account_risk_totals(
        env,
        &mut cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );

    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, &account, PositionSides::BOTH, false);

    // Post-liq totals: empty debt → account cleanup; residual bad debt → socialize.
    apply::check_bad_debt_after_liquidation(
        env,
        &mut cache,
        account_id,
        &account,
        post_totals.total_collateral,
        post_totals.total_debt,
    );
    cache.emit_position_batch(account_id, &account);
}

fn validate_liquidation_inputs(
    env: &Env,
    account: &Account,
    liquidator: &Address,
    aggregated: &AggregatedPayments,
) {
    validation::require_non_empty_payments(env, aggregated);

    // Owner only; a registered delegate may liquidate an account it manages.
    assert_with_error!(
        env,
        account.owner != *liquidator,
        CollateralError::SelfLiquidationNotAllowed
    );
}

/// Socializes residual bad debt when eligible; removes the account on success.
pub(crate) fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    // Same risk-totals + threshold as the post-liquidation cleanup path.
    let mut cache = Cache::new(env);
    let account = storage::get_account(env, account_id);

    assert_with_error!(
        env,
        !account.borrow_positions.is_empty(),
        CollateralError::DebtPositionNotFound
    );

    let totals = risk::calculate_account_risk_totals(
        env,
        &mut cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );

    if !is_socializable_bad_debt(totals.total_debt, totals.total_collateral) {
        panic_with_error!(env, CollateralError::CannotCleanBadDebt);
    }

    bad_debt::execute_bad_debt_cleanup(
        env,
        &mut cache,
        account_id,
        &account,
        totals.total_debt.raw(),
        totals.total_collateral.raw(),
    );
}
