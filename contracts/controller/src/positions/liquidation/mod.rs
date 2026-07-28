//! Liquidation and residual bad-debt socialization.
//!
//! Pipeline: plan (HF < 1, price, normalize) → apply (repay then seize) →
//! optional bad-debt cleanup. Permissionless keepers; not gated by global pause.
//! Spoke pause blocks both sides: paused debt accepts no repay tokens and
//! paused collateral is not seizable (frozen/delisted legs stay seizable).
//! See `docs/reference/invariants.md` §3.3.

use crate::risk;
use common::validation::require_non_empty_payments;
mod apply;
mod bad_debt;
pub(crate) mod curve;
pub(crate) mod math;
mod plan;

pub(crate) use plan::execute_liquidation;

use common::errors::CollateralError;
use common::math::fp::Wad;
use common::types::{Account, AggregatedPayments, HubPayment};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use self::curve::is_socializable_bad_debt;
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::payments;
use crate::positions::{persist_position_flow, PositionSides};
use crate::risk::validation;
use crate::storage;

/// Auth, plan, transfer repay + seize, persist both sides, then residual bad debt.
///
/// Uses `persist_position_flow` rather than `finalize_position_flow`: the
/// bad-debt check has to run between persisting and emitting, since it may
/// delete the account outright.
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

    require_non_empty_payments(env, &result.repaid);

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
        &account.supply_positions,
        &account.borrow_positions,
    );

    persist_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::BOTH,
        false,
    );

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
    require_non_empty_payments(env, aggregated);

    // Owner only; a registered delegate may liquidate an account it manages.
    assert_with_error!(
        env,
        account.owner != *liquidator,
        CollateralError::SelfLiquidationNotAllowed
    );
}

/// Caller auth and flash-loan guard, then the standalone socialization path.
pub(crate) fn process_clean_bad_debt(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    clean_bad_debt_standalone(env, account_id);
}

/// Eligibility rule applied to an account's risk totals before socialization.
enum BadDebtGate {
    /// Underwater and collateral at or below the dust threshold.
    DustCapped,
    /// Underwater only, no dust cap. Seizure moves collateral shares to revenue
    /// and writes debt down through the supply index — neither step transfers
    /// tokens, so a frozen or clawed collateral leg does not block the cleanup.
    Insolvent,
}

impl BadDebtGate {
    fn admits(&self, total_debt: Wad, total_collateral: Wad) -> bool {
        match self {
            Self::DustCapped => is_socializable_bad_debt(total_debt, total_collateral),
            Self::Insolvent => total_debt > total_collateral,
        }
    }
}

/// Socializes residual bad debt when `gate` admits the account; removes it on success.
fn socialize_bad_debt(env: &Env, account_id: u64, gate: BadDebtGate) {
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
        &account.supply_positions,
        &account.borrow_positions,
    );

    assert_with_error!(
        env,
        gate.admits(totals.total_debt, totals.total_collateral),
        CollateralError::CannotCleanBadDebt
    );

    bad_debt::execute_bad_debt_cleanup(
        env,
        &mut cache,
        account_id,
        &account,
        totals.total_debt.raw(),
        totals.total_collateral.raw(),
    );
}

/// Permissionless socialization: same risk totals and threshold as the
/// post-liquidation cleanup path.
pub(crate) fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    socialize_bad_debt(env, account_id, BadDebtGate::DustCapped);
}

/// Flash-loan guard, then governance socialization gated on plain insolvency, so
/// a large-collateral account whose collateral cannot be seized can still be retired.
pub(crate) fn process_force_socialize_bad_debt(env: &Env, account_id: u64) {
    validation::require_not_flash_loaning(env);
    socialize_bad_debt(env, account_id, BadDebtGate::Insolvent);
}
