//! Builds the liquidation plan (priced repay/seize legs and bonus).
//!
//! Gates: non-empty debt, debt-leg pause preflight, HF < 1. Then normalizes
//! repay amounts and pro-rata seizures. Pure relative to pool transfers —
//! apply owns money movement.

use crate::risk;
use common::errors::{CollateralError, SpokeError};
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

/// Prices positions, enforces HF < 1, returns the full repay/seize plan.
pub(crate) fn build_liquidation_plan(
    env: &Env,
    account: &Account,
    aggregated_debt: &Vec<HubPayment>,
    cache: &mut Cache,
) -> LiquidationPlan {
    // Debt-free accounts never liquidate; skip pricing a saturated HF.
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }

    // Twin of the per-transfer gate in apply: listed debt must not be paused.
    // Missing listing is not treated as paused (pricing fails closed later).
    for (hub_asset, _) in aggregated_debt.iter() {
        let debt_paused = cache
            .cached_spoke_asset(account.spoke_id, &hub_asset)
            .is_some_and(|c| c.paused);
        assert_with_error!(env, !debt_paused, SpokeError::SpokeAssetPaused);
    }

    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    );
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
