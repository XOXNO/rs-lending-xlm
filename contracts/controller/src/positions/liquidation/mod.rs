//! Liquidation and residual bad-debt cleanup.
//!
//! Pipeline: plan (HF gate, price, normalize) → apply (repay then seize) →
//! optional bad-debt socialization. Requires HF < 1 to liquidate.
//!
//! Not gated by `#[when_not_paused]`: keepers can liquidate and clean bad debt
//! while the contract is paused. Spoke-asset pause blocks inbound debt repay
//! tokens; seizure of paused collateral remains allowed.
//!
//! `result.refunds` from the plan is informational only — the liquidator only
//! transfers post-normalization repay amounts (never over-pulled).

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
    /// Liquidates an unhealthy account: repays selected debt and seizes bonused
    /// collateral. Requires the target's health factor to be below one.
    ///
    /// Not blocked by the global pause flag. Permissionless for any non-owner
    /// liquidator (a registered delegate may liquidate an account it manages).
    ///
    /// # Arguments
    /// * `liquidator` - pays debt and receives seized collateral; must authorize.
    ///   Cannot be the account owner.
    /// * `account_id` - existing undercollateralized account.
    /// * `debt_payments` - `(hub-asset, amount)` debt legs to repay; positive amounts.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `InvalidPayments` - the debt payment list (or the resulting repayment set) is empty.
    /// * `AmountMustBePositive` - a leg amount is not strictly positive.
    /// * `SelfLiquidationNotAllowed` - `liquidator` is the account owner.
    /// * `SpokeAssetPaused` - a repaid debt leg's listing is paused (paused
    ///   listings accept no inbound tokens; seizure of paused collateral stays
    ///   allowed).
    /// * `HealthFactorTooHigh` - the account is still at or above HF of one.
    /// * A non-market debt asset reverts `OracleNotConfigured` / `PoolNotInitialized`
    ///   on the fail-closed pricing path.
    ///
    /// # Events
    /// * A `LiquidationEvent` (liquidator, account, repaid USD, bonus bps) and a
    ///   position-batch event for the account's updated legs.
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    /// Socializes small residual bad debt by seizing all remaining supply and
    /// debt shares into the pool. Permissionless; `caller` auth is for
    /// accountability only (no proceeds to the caller).
    ///
    /// Not blocked by the global pause flag. Same eligibility predicate as the
    /// post-liquidation automatic cleanup path.
    ///
    /// # Arguments
    /// * `caller` - accountable initiator; must authorize.
    /// * `account_id` - account with socializable residual bad debt.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` - a flash loan or strategy is mid-execution.
    /// * `DebtPositionNotFound` - the account carries no debt.
    /// * `CannotCleanBadDebt` - not eligible (debt not socializable residual).
    ///
    /// # Events
    /// * A `CleanBadDebtEvent` and account removal (no position-batch if gone).
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

    // Same fail-closed oracle staleness/tolerance as every other risk path.
    let mut cache = Cache::new(env);

    validate_liquidation_inputs(env, &account, liquidator, &aggregated);

    let liquidation_plan = plan::build_liquidation_plan(env, &account, &aggregated, &mut cache);
    // Refunds in the plan are not transferred; only `result.repaid` is pulled.
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

/// Rejects empty payments and owner self-liquidation before pricing.
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
