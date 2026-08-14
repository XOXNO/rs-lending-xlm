use super::curve::{LiquidationCurve, LiquidationSnapshot};
use crate::risk;
use common::errors::CollateralError;
use common::math::fp::Wad;
use common::types::{Account, HubPayment, LiquidationResult};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::context::Cache;
use crate::positions::liquidation::math::*;
use crate::positions::{enforce_spoke_asset_flags, FreezePolicy};

/// Computes what liquidating `account` with `raw_payments` would produce, without persisting
/// anything: builds a `LiquidationPlan` and converts it into the returned `LiquidationResult`.
pub(crate) fn execute_liquidation(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    cache: &mut Cache,
) -> LiquidationResult {
    build_liquidation_plan(env, account, raw_payments, cache).into_result()
}

/// Builds and validates a `LiquidationPlan` for `account` from `raw_payments`: computes risk
/// totals, sizes the repayment against the liquidation curve's ideal close amount, and derives
/// the pro-rata collateral seizure. Panics with `CollateralError::HealthFactorTooHigh` when the
/// account has no debt or its health factor is at least one WAD, and enforces spoke pause/freeze
/// flags on every payment and seizure asset.
pub(crate) fn build_liquidation_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    cache: &mut Cache,
) -> LiquidationPlan {
    if account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::HealthFactorTooHigh);
    }

    for (hub_asset, _) in raw_payments.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &hub_asset,
            FreezePolicy::AllowOnExit,
        );
    }

    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
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
        raw_payments,
        &snap,
        bonus_bounds,
        &curve,
        cache,
    );

    let seized_collaterals =
        calculate_seized_collateral(env, account, totals.total_collateral, &repayment, cache);

    for entry in seized_collaterals.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::AllowOnExit,
        );
    }

    let plan = LiquidationPlan {
        repayment,
        seized: seized_collaterals,
    };
    plan.validate(env);
    plan
}
