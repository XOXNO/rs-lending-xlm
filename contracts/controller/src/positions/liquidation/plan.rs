use super::curve::{LiquidationCurve, LiquidationSnapshot};
use crate::risk;
use common::errors::CollateralError;
use common::math::fp::Wad;
use common::types::{Account, HubPayment};
use soroban_sdk::{assert_with_error, panic_with_error, Env, Vec};

use crate::context::Context;
use crate::positions::liquidation::math::*;
use crate::positions::{enforce_spoke_asset_flags, FreezePolicy};

/// Builds a normalized repayment and pro-rata seizure plan for debt with HF < 1 WAD.
/// Repayment rejects `paused` but permits `frozen`; seizure rejects only `no_seize`.
pub(crate) fn build_liquidation_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    cache: &mut Context,
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
        weighted_collateral: totals.weighted_collateral,
        proportion_seized,
        hf: totals.health_factor,
    };

    let curve = LiquidationCurve::from_config(&cache.spoke_config(account.spoke_id));
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

    // Pro-rata seizure must not inherit user pause flags (ADR-0008).
    for entry in seized_collaterals.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::SeizureLeg,
        );
    }

    let plan = LiquidationPlan {
        repayment,
        seized: seized_collaterals,
    };
    plan.validate(env);
    plan
}
