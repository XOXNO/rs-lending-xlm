//! Liquidation pipeline, one phase per file: `plan` builds a validated
//! `LiquidationPlan` (shared with the estimate view), `apply` executes
//! repayments and seizures, `math` and `curve` hold the pure arithmetic
//! (`curve` is certora-pinned), `bad_debt` socializes and removes insolvent
//! accounts.

use crate::risk;
use common::validation::require_non_empty_payments;
// `apply` and `bad_debt` are crate-visible so the Certora spec layer can drive
// the real seizure and cleanup entry points. When they were private the rules
// could only reach one level below what actually runs.
pub(crate) mod apply;
pub(crate) mod bad_debt;
pub(crate) mod curve;
pub(crate) mod math;
mod plan;

pub(crate) use math::split_seized_shares;
pub(crate) use plan::execute_liquidation;

use common::errors::{CollateralError, GenericError, SpokeError};
use common::types::{Account, HubPayment, PositionMode, SeizeMode};
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use self::curve::is_socializable_bad_debt;
use crate::account;
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::validation;
use crate::risk::AccountRiskTotals;
use crate::storage;

/// Liquidates `account_id`'s undercollateralized debt: builds a plan from `debt_payments`,
/// applies the repayments, then seizes collateral scaled down to match whatever was actually
/// received. Requires `liquidator` to authorize the call and socializes any bad debt left on
/// the account once seizure completes. Owners may liquidate their own account; only crediting
/// seized collateral back to the account being liquidated is rejected (see
/// `resolve_seize_receiver`).
///
/// `seize_mode` decides how the liquidator takes delivery. `Transfer` pays them in underlying
/// out of pool cash. `Credit` instead moves the seized supply shares to a controller account,
/// so the only token movement in the whole call is the liquidator's own repayment — which is
/// what lets a liquidation clear a market with no spare cash. Returns the receiving account id
/// in credit mode, `0` in transfer mode.
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

    let mut cache = Cache::new(env);

    validate_liquidation_inputs(env, debt_payments);

    // Resolved up front so an unusable receiving account fails before any token moves.
    let mut receiver = resolve_seize_receiver(
        env, liquidator, account_id, &account, seize_mode, &mut cache,
    );

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

    // Report what the pool actually received, not what the plan intended to
    // collect. An under-delivering debt token makes the two differ, and the
    // planned figure would overstate the debt retired.
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

    // Credit mode writes two accounts, so it publishes a second position batch. Emitted here,
    // between the liquidated account's batch and any bad-debt cleanup, so the ordering an
    // indexer sees stays fully determined.
    if let Some((receiver_id, receiving_account)) = &receiver {
        apply::record_share_credit_updates(env, receiving_account, &seized, &mut cache);
        finalize_position_flow(
            env,
            *receiver_id,
            receiving_account,
            &mut cache,
            PositionSides::SUPPLY,
            false,
        );
    }

    apply::check_bad_debt_after_liquidation(env, &mut cache, account_id, &account, &post_totals);

    receiver.map_or(0, |(id, _)| id)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_zero_threshold.rs"]
mod zero_threshold_tests;

/// Resolves where seized collateral is delivered.
///
/// `Transfer` yields `None`: the pool pays the liquidator in underlying. `Credit(0)` creates a
/// fresh account owned by the liquidator; `Credit(id)` uses an existing one, which must belong
/// to the liquidator (directly or through an active delegate), sit in the liquidated account's
/// spoke, be in `PositionMode::Normal`, and not be the liquidated account itself.
///
/// The spoke must match because the credited shares are that spoke's supply and an account's
/// spoke binding is what supplies the risk configuration for every position it holds; letting
/// them diverge would move collateral into a different risk regime. The mode must be `Normal`
/// because strategy modes carry invariants this path does not establish.
fn resolve_seize_receiver(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    account: &Account,
    seize_mode: SeizeMode,
    cache: &mut Cache,
) -> Option<(u64, Account)> {
    let requested = match seize_mode {
        SeizeMode::Transfer => return None,
        SeizeMode::Credit(id) => id,
    };

    if requested == 0 {
        return Some(account::create_account(
            env,
            liquidator,
            account.spoke_id,
            PositionMode::Normal,
            cache,
        ));
    }

    // Crediting the liquidated account would hand its own collateral straight back and undo
    // the seizure.
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

/// Rejects empty `raw_payments`.
fn validate_liquidation_inputs(env: &Env, raw_payments: &Vec<HubPayment>) {
    require_non_empty_payments(env, raw_payments);
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
