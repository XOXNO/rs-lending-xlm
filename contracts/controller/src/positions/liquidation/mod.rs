//! Liquidation planning, repayment, seizure, and bad-debt cleanup.
//! Execution and estimates share the same validated plan and arithmetic.

use crate::risk;
use common::validation::require_non_empty_payments;
// Crate visibility lets Certora exercise seizure, cleanup, and curve arithmetic directly.
pub(crate) mod apply;
pub(crate) mod bad_debt;
pub(crate) mod curve;
pub(crate) mod math;
mod plan;

pub(crate) use math::split_seized_shares;
pub(crate) use plan::build_liquidation_plan;

use common::errors::{CollateralError, GenericError, SpokeError};
use common::types::{Account, HubPayment, PositionMode, SeizeMode};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use self::curve::is_socializable_bad_debt;
use crate::account;
use crate::account::SpokeAdmission;
use crate::context::Context;
use crate::events::LiquidationEvent;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::validation;
use crate::storage;

/// Repays unhealthy debt, sizes seizure to measured receipts, and socializes
/// eligible residual bad debt. Owners may self-liquidate but cannot credit
/// seized shares back to the liquidated account.
///
/// `Transfer` pays underlying from pool cash. `Credit` moves supply shares,
/// requiring no collateral cash. Returns the credit receiver id, or zero for transfer.
pub(crate) fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
    seize_mode: SeizeMode,
) -> u64 {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    let mut cache = Context::new(env);

    require_non_empty_payments(env, debt_payments);

    // Reject an unusable receiver before moving tokens.
    let mut receiver = resolve_seize_receiver(
        env, liquidator, account_id, &account, seize_mode, &mut cache,
    );

    // Share payment normalization and positivity checks with the estimate view.
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

    // Under-delivering debt tokens must reduce the collateral awarded.
    let repay_usd = math::sum_repaid_usd(env, &result.repaid);
    let seized = math::scale_seizures_to_received(env, &result.seized, received_usd, repay_usd);
    match &mut receiver {
        None => {
            apply::apply_liquidation_seizures(env, liquidator, &mut account, &seized, &mut cache)
        }
        Some((_, receiving_account)) => {
            apply::require_credit_position_limit(env, receiving_account, &seized);
            apply::apply_liquidation_share_credit(
                env,
                &mut account,
                receiving_account,
                &seized,
                &mut cache,
            );
        }
    }

    // Report measured receipt value, capped per planned repayment leg.
    LiquidationEvent {
        liquidator: liquidator.clone(),
        account_id,
        repaid_usd_wad: received_usd.raw(),
        bonus_bps: result.bonus_bps,
    }
    .publish(env);

    let post_totals = risk::calculate_account_risk_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    // Event order: LiquidationEvent, liquidated account's UpdatePositionBatchEvent,
    // optional receiver batch, then CleanBadDebtEvent. Cleanup emits no position deltas.
    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Both,
        false,
    );

    if let Some((receiver_id, receiving_account)) = &receiver {
        apply::record_share_credit_updates(env, receiving_account, &seized, &mut cache);
        finalize_position_flow(
            env,
            *receiver_id,
            receiving_account,
            &mut cache,
            PositionSides::Supply,
            false,
        );
    }

    apply::check_bad_debt_after_liquidation(env, &mut cache, account_id, &account, &post_totals);

    receiver.map_or(0, |(id, _)| id)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_zero_threshold.rs"]
mod zero_threshold_tests;

/// Resolves an authorized credit receiver, or `None` for underlying transfer.
/// `Credit(0)` creates an account owned by the liquidator.
///
/// Credit requires a different account in the same spoke and normal mode:
/// another spoke changes the risk regime; strategy modes need additional invariants.
fn resolve_seize_receiver(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    account: &Account,
    seize_mode: SeizeMode,
    cache: &mut Context,
) -> Option<(u64, Account)> {
    let requested = match seize_mode {
        SeizeMode::Transfer => return None,
        SeizeMode::Credit(id) => id,
    };

    if requested == 0 {
        return Some(account::create_account_with(
            env,
            liquidator,
            account.spoke_id,
            PositionMode::Normal,
            cache,
            SpokeAdmission::AllowDeprecated,
        ));
    }

    // Crediting the same account would undo the seizure.
    assert_with_error!(
        env,
        requested != account_id,
        CollateralError::SelfLiquidationNotAllowed
    );

    let receiver = storage::get_account(env, requested);
    account::require_owner_or_delegate(env, requested, liquidator, &receiver.owner);
    assert_with_error!(
        env,
        receiver.spoke_id == account.spoke_id,
        SpokeError::SpokeMismatch
    );
    assert_with_error!(
        env,
        receiver.mode == PositionMode::Normal,
        GenericError::AccountModeMismatch
    );

    Some((requested, receiver))
}

/// Authorizes permissionless dust-gated cleanup outside flash loans.
pub(crate) fn process_clean_bad_debt(env: &Env, caller: &Address, account_id: u64) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    clean_bad_debt_standalone(env, account_id);
}

/// Admission condition for bad-debt socialization.
#[derive(Clone, Copy, PartialEq)]
enum BadDebtGate {
    /// Permissionless: insolvent *and* collateral at or below the dust threshold.
    DustCapped,
    /// Owner-only: insolvent alone, with no cap on the collateral left behind.
    InsolventOnly,
}

/// Requires open debt and the selected insolvency gate, then cleans up the account.
fn socialize_bad_debt(env: &Env, account_id: u64, gate: BadDebtGate) {
    let mut cache = Context::new(env);
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

    let admits = match gate {
        BadDebtGate::DustCapped => {
            is_socializable_bad_debt(totals.total_debt, totals.total_collateral)
        }
        BadDebtGate::InsolventOnly => totals.total_debt > totals.total_collateral,
    };
    assert_with_error!(env, admits, CollateralError::CannotCleanBadDebt);

    bad_debt::execute_bad_debt_cleanup(env, &mut cache, account_id, &account, &totals);
}

/// Socializes insolvent debt when remaining collateral is at or below the dust cap.
pub(crate) fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    socialize_bad_debt(env, account_id, BadDebtGate::DustCapped);
}

/// Socializes debt exceeding collateral without a dust cap, outside flash loans.
pub(crate) fn process_force_socialize_bad_debt(env: &Env, account_id: u64) {
    validation::require_not_flash_loaning(env);
    socialize_bad_debt(env, account_id, BadDebtGate::InsolventOnly);
}
