//! Liquidation and bad-debt cleanup. Liquidation requires HF < 1.

use crate::risk;
mod apply;
mod bad_debt;
pub mod math;
mod plan;

pub(crate) use plan::execute_liquidation;

use common::errors::CollateralError;
use common::types::{Account, HubAssetKey};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Vec};

use self::math::is_socializable_bad_debt;
use super::{persist_account_positions, PositionSides};
use crate::context::Cache;
use crate::events::LiquidationEvent;
use crate::positions::{AggregatedPayments, HubPayment};
use crate::{
    payments as utils, risk::validation, storage, Controller, ControllerArgs, ControllerClient,
};

#[contractimpl]
impl Controller {
    pub fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) {
        process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    pub fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        // Auth binds the cleanup to an accountable caller; the operation
        // itself is permissionless.
        caller.require_auth();
        validation::require_not_flash_loaning(&env);
        clean_bad_debt_standalone(&env, account_id);
    }
}

pub fn process_liquidation(
    env: &Env,
    liquidator: &Address,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
) {
    liquidator.require_auth();
    validation::require_not_flash_loaning(env);

    let mut account = storage::get_account(env, account_id);

    let aggregated = utils::aggregate_positive_payments(env, debt_payments);

    // Pricing below uses the same fail-closed staleness/tolerance checks as
    // every other risk computation; liquidation has no distinct oracle path.
    let mut cache = Cache::new(env);

    validate_liquidation_inputs(env, &account, liquidator, &aggregated);

    let liquidation_plan = plan::build_liquidation_plan(env, &account, &aggregated, &mut cache);
    // `result.refunds` is informational: the liquidator only ever transfers
    // the post-normalization repaid amounts, so no refund transfer exists here.
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

    // Reuse the post-liquidation account snapshot for bad-debt cleanup.
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

    // The guard covers only the owner; a registered delegate liquidating an
    // account it manages remains allowed (deliberate).
    assert_with_error!(
        env,
        account.owner != *liquidator,
        CollateralError::SelfLiquidationNotAllowed
    );

    // Debt assets are priced and repaid downstream; a non-market asset reverts
    // `OracleNotConfigured`/`PoolNotInitialized` there.
}

/// Socializes small residual bad debt by seizing all collateral and debt shares.
pub fn clean_bad_debt_standalone(env: &Env, account_id: u64) {
    // Success removes the account; failure reverts atomically, so no keep-alive is needed.
    // Uses the same risk-totals computation as the post-liquidation path, so the
    // bad-debt threshold check stays consistent between both entry points.
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
