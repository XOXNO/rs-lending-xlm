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
use common::types::{Account, HubPayment};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use self::curve::is_socializable_bad_debt;
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::validation;
use crate::storage;

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

    apply::check_bad_debt_after_liquidation(
        env,
        &mut cache,
        account_id,
        &account,
        post_totals.total_collateral,
        post_totals.total_debt,
    );
}

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
    fn admits(&self, total_debt: Wad, total_collateral: Wad) -> bool {
        match self {
            Self::DustCapped => is_socializable_bad_debt(total_debt, total_collateral),
            Self::Insolvent => total_debt > total_collateral,
        }
    }
}

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

pub(crate) fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    socialize_bad_debt(env, account_id, BadDebtGate::DustCapped);
}

pub(crate) fn process_force_socialize_bad_debt(env: &Env, account_id: u64) {
    validation::require_not_flash_loaning(env);
    socialize_bad_debt(env, account_id, BadDebtGate::Insolvent);
}
