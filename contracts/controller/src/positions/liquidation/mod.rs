//! Liquidation pipeline, one phase per file: `plan` builds a validated
//! `LiquidationPlan` (shared with the estimate view), `apply` executes
//! repayments and seizures, `math` and `curve` hold the pure arithmetic
//! (`curve` is certora-pinned), `bad_debt` socializes and removes insolvent
//! accounts.

use crate::risk;
use common::validation::require_non_empty_payments;
mod apply;
mod bad_debt;
pub(crate) mod curve;
pub(crate) mod math;
mod plan;

pub(crate) use plan::execute_liquidation;

use common::errors::CollateralError;
use common::types::{Account, HubPayment};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use self::curve::is_socializable_bad_debt;
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::validation;
use crate::risk::AccountRiskTotals;
use crate::storage;

/// Liquidates `account_id`'s undercollateralized debt: builds a plan from `debt_payments`,
/// applies the repayments, then seizes collateral scaled down to match whatever was actually
/// received. Requires `liquidator` to authorize the call, rejects self-liquidation, and
/// socializes any bad debt left on the account once seizure completes.
pub(crate) fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    let mut cache = Cache::new(env);

    validate_liquidation_inputs(env, &account, liquidator, debt_payments);

    // The plan is the single normalization point: it merges and positivity-checks
    // the raw payments, so the estimate view and this entry point share one path.
    let liquidation_plan = plan::build_liquidation_plan(env, &account, debt_payments, &mut cache);

    let result = liquidation_plan.into_result();

    require_non_empty_payments(env, &result.repaid);

    let received_usd = apply::apply_liquidation_repayments(
        env,
        liquidator,
        &mut account,
        &result.repaid,
        &mut cache,
    );

    // Collateral is sized from the repayment the plan intended to collect. If a
    // debt token delivered less than was sent, shrink the seizure to match, or
    // the liquidator keeps collateral they did not pay for.
    let repay_usd = math::sum_repaid_usd(env, &result.repaid);
    let seized = math::scale_seizures_to_received(env, &result.seized, received_usd, repay_usd);
    apply::apply_liquidation_seizures(env, liquidator, &mut account, &seized, &mut cache);

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

    // Event order is a contract: finalize persists both sides and publishes
    // UpdatePositionBatchEvent with the post-liquidation positions; bad-debt
    // cleanup afterwards records no position deltas — it only publishes
    // CleanBadDebtEvent and removes the account entry.
    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::BOTH,
        false,
    );

    apply::check_bad_debt_after_liquidation(env, &mut cache, account_id, &account, &post_totals);
}

/// Rejects empty `raw_payments` and self-liquidation, where `liquidator` is the account's own
/// owner.
fn validate_liquidation_inputs(
    env: &Env,
    account: &Account,
    liquidator: &Address,
    raw_payments: &Vec<HubPayment>,
) {
    require_non_empty_payments(env, raw_payments);

    assert_with_error!(
        env,
        account.owner != *liquidator,
        CollateralError::SelfLiquidationNotAllowed
    );
}

/// Socializes `account_id`'s bad debt under the dust-threshold gate; requires `caller` to
/// authorize the call and reverts while a flash loan is in progress.
pub(crate) fn process_clean_bad_debt(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    clean_bad_debt_standalone(env, account_id);
}

enum BadDebtGate {
    DustCapped,

    Insolvent,
}

impl BadDebtGate {
    /// Returns whether `totals` satisfies this gate's bad-debt condition: `DustCapped` also
    /// requires collateral at or below the dust threshold, `Insolvent` only requires debt
    /// exceeding collateral.
    fn admits(&self, totals: &AccountRiskTotals) -> bool {
        match self {
            Self::DustCapped => {
                is_socializable_bad_debt(totals.total_debt, totals.total_collateral)
            }
            Self::Insolvent => totals.total_debt > totals.total_collateral,
        }
    }
}

/// Loads `account_id`, asserts it has open debt and that its risk totals satisfy `gate`, then
/// runs bad-debt cleanup to seize its remaining positions and remove the account entry.
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
        gate.admits(&totals),
        CollateralError::CannotCleanBadDebt
    );

    bad_debt::execute_bad_debt_cleanup(env, &mut cache, account_id, &account, &totals);
}

/// Applies the dust-threshold bad-debt gate to `account_id`.
pub(crate) fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    socialize_bad_debt(env, account_id, BadDebtGate::DustCapped);
}

/// Applies the insolvency bad-debt gate to `account_id` — total debt exceeding total collateral,
/// without the dust-threshold cap — and reverts while a flash loan is in progress.
pub(crate) fn process_force_socialize_bad_debt(env: &Env, account_id: u64) {
    validation::require_not_flash_loaning(env);
    socialize_bad_debt(env, account_id, BadDebtGate::Insolvent);
}
