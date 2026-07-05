//! Builds the liquidation plan: priced repay/seize legs and the bonus, gated on
//! the target account's health factor being below one.

use crate::risk;
use common::errors::CollateralError;
use common::math::fp::Wad;
use common::types::{Account, LiquidationResult};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::context::Cache;
use crate::positions::liquidation::math::*;
use crate::positions::HubPayment;

pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    aggregated_debt: &Vec<HubPayment>,
    cache: &mut Cache,
) -> LiquidationResult {
    build_liquidation_plan(env, account, aggregated_debt, cache).into_result()
}

pub(super) fn build_liquidation_plan(
    env: &Env,
    account: &Account,
    aggregated_debt: &Vec<HubPayment>,
    cache: &mut Cache,
) -> LiquidationPlan {
    // One totals pass feeds both the HF gate and the snapshot. A debt-free
    // account carries a saturated health factor that fails the `hf < ONE` gate,
    // but the early panic skips pricing it.
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }
    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );
    // dimensional: totals are Wad<USD>; health_factor is Wad<1>.
    assert_with_error!(
        env,
        totals.health_factor < Wad::ONE,
        CollateralError::HealthFactorTooHigh
    );

    let (proportion_seized, bonus_bounds) = calculate_seizure_proportions(
        env,
        account,
        totals.total_collateral,
        totals.weighted_collateral,
        cache,
    );

    let snap = LiquidationSnapshot {
        total_debt: totals.total_debt,
        total_collateral: totals.total_collateral,
        weighted_coll: totals.weighted_collateral,
        proportion_seized,
        hf: totals.health_factor,
    };

    let curve = LiquidationCurve::resolve(cache, account.spoke_id);
    let repayment = normalize_repayment_plan(
        env,
        account,
        aggregated_debt,
        &snap,
        bonus_bounds,
        &curve,
        cache,
    );

    let seized_collaterals =
        calculate_seized_collateral(env, account, totals.total_collateral, &repayment, cache);

    let plan = LiquidationPlan {
        repayment,
        seized: seized_collaterals,
    };
    plan.validate(env);
    plan
}
