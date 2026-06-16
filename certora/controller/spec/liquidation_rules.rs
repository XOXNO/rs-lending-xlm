/// Liquidation invariant rules: debt/collateral reduction, bonus bounds, seizure, fees.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::constants::{BPS, WAD};
use crate::types::AccountPositionType;
use common::math::fp::{Bps, Wad};
use common::math::fp_core::{mul_div_floor, mul_div_half_up};

// Caps debt payment inputs to realistic position sizes and avoids i128 overflow paths.
const MAX_DEBT_AMOUNT_RAW: i128 = 1_000_000_000_000;

/// Liquidation strictly decreases scaled debt for the repaid asset.
#[rule]
fn liquidation_strictly_decreases_debt_for_repaid_asset(
    e: Env,
    liquidator: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let account_id: u64 = 1;

    cvlr_assume!(debt_amount > 0);
    cvlr_assume!(debt_amount <= MAX_DEBT_AMOUNT_RAW);

    let borrow_pre =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_asset);
    cvlr_assume!(borrow_pre.is_some());
    let scaled_debt_before = borrow_pre.unwrap().scaled_amount_ray;
    cvlr_assume!(scaled_debt_before > 0);

    let mut payments: Vec<(Address, i128)> = Vec::new(&e);
    payments.push_back((debt_asset.clone(), debt_amount));

    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    let borrow_post =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_asset);
    match borrow_post {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < scaled_debt_before),
        None => cvlr_assert!(true),
    }
}

/// Liquidation strictly decreases scaled collateral for the seized asset.
#[rule]
fn liquidation_strictly_decreases_collateral_for_seized_asset(
    e: Env,
    liquidator: Address,
    collateral_asset: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let account_id: u64 = 1;

    cvlr_assume!(debt_amount > 0);
    cvlr_assume!(debt_amount <= MAX_DEBT_AMOUNT_RAW);

    let supply_pre = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_asset,
    );
    cvlr_assume!(supply_pre.is_some());
    let scaled_col_before = supply_pre.unwrap().scaled_amount_ray;
    cvlr_assume!(scaled_col_before > 0);

    let borrow_pre =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_asset);
    cvlr_assume!(borrow_pre.is_some());
    cvlr_assume!(borrow_pre.unwrap().scaled_amount_ray > 0);

    let mut payments: Vec<(Address, i128)> = Vec::new(&e);
    payments.push_back((debt_asset, debt_amount));

    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    let supply_post = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_asset,
    );
    match supply_post {
        Some(pos) => cvlr_assert!(pos.scaled_amount_ray < scaled_col_before),
        None => cvlr_assert!(true),
    }
}

/// Dynamic liquidation bonus stays within [base_bonus, max_bonus] for liquidatable HF.
#[rule]
fn bonus_bounded(
    e: Env,
    hf_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
    target_wad: i128,
) {
    cvlr_assume!(base_bonus_bps >= 0);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= BPS);
    cvlr_assume!(hf_wad >= 0);
    cvlr_assume!(hf_wad < WAD);
    cvlr_assume!(target_wad > 0 && target_wad <= 2 * WAD);

    // Real production bonus math (NOT the certora summary `calculate_linear_bonus`,
    // which would assume the very bounds asserted here). Proves the production
    // function keeps the bonus in [base, max] for any liquidation target.
    let bonus = crate::positions::liquidation_math::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf_wad),
        Bps::from(base_bonus_bps),
        Bps::from(max_bonus_bps),
        Wad::from(target_wad),
    );

    cvlr_assert!(bonus.raw() <= max_bonus_bps);
    cvlr_assert!(bonus.raw() >= base_bonus_bps);
}

/// Per-account max bonus keeps effective_threshold * (1 + bonus) <= 100%.
#[rule]
fn derived_bonus_respects_threshold(e: Env, proportion_seized_wad: i128) {
    cvlr_assume!(proportion_seized_wad > 0);
    cvlr_assume!(proportion_seized_wad <= WAD);

    let max = crate::positions::liquidation_math::max_bonus_for_threshold(
        &e,
        Wad::from(proportion_seized_wad),
    );

    let mut eff_thr_bps = (proportion_seized_wad * BPS + (WAD - 1)) / WAD;
    if eff_thr_bps < 1 {
        eff_thr_bps = 1;
    }
    if eff_thr_bps > BPS {
        eff_thr_bps = BPS;
    }

    cvlr_assert!(eff_thr_bps * (BPS + max.raw()) <= BPS * BPS);
}

/// Each asset's seizure is proportional to its share of total collateral value.
#[rule]
fn seizure_proportional(
    e: Env,
    total_seizure_usd_wad: i128,
    asset_a_value_wad: i128,
    asset_b_value_wad: i128,
) {
    cvlr_assume!(total_seizure_usd_wad > 0);
    cvlr_assume!(asset_a_value_wad > 0);
    cvlr_assume!(asset_b_value_wad > 0);

    let total_collateral_wad = asset_a_value_wad + asset_b_value_wad;
    cvlr_assume!(total_collateral_wad > 0);
    cvlr_assume!(total_seizure_usd_wad <= total_collateral_wad);

    let share_a_wad = mul_div_half_up(&e, asset_a_value_wad, WAD, total_collateral_wad);
    let seizure_a = mul_div_half_up(&e, total_seizure_usd_wad, share_a_wad, WAD);

    let share_b_wad = mul_div_half_up(&e, asset_b_value_wad, WAD, total_collateral_wad);
    let seizure_b = mul_div_half_up(&e, total_seizure_usd_wad, share_b_wad, WAD);

    cvlr_assert!(seizure_a >= 0);
    cvlr_assert!(seizure_b >= 0);
    cvlr_assert!(seizure_a + seizure_b <= total_seizure_usd_wad + 1);

    if asset_a_value_wad > asset_b_value_wad {
        cvlr_assert!(seizure_a >= seizure_b);
    }
}

/// Protocol fee is charged on the bonus portion only, not the full seizure.
#[rule]
fn protocol_fee_on_bonus_only(
    e: Env,
    seizure_amount: i128,
    bonus_bps: i128,
    liquidation_fees_bps: i128,
) {
    cvlr_assume!(seizure_amount > 0);
    cvlr_assume!(seizure_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(bonus_bps > 0);
    cvlr_assume!(bonus_bps <= BPS);
    cvlr_assume!(liquidation_fees_bps >= 0);
    cvlr_assume!(liquidation_fees_bps <= BPS);

    let one_plus_bonus_wad = WAD + mul_div_half_up(&e, bonus_bps, WAD, BPS);
    let base_amount = mul_div_floor(&e, seizure_amount, WAD, one_plus_bonus_wad);
    let bonus_amount = seizure_amount - base_amount;
    let protocol_fee = mul_div_half_up(&e, bonus_amount, liquidation_fees_bps, BPS);

    cvlr_assert!(protocol_fee <= bonus_amount);
    cvlr_assert!(protocol_fee >= 0);

    if liquidation_fees_bps == 0 {
        cvlr_assert!(protocol_fee == 0);
    }

    cvlr_assert!(protocol_fee < seizure_amount);
}

/// Ideal repayment is positive, bounded by debt, and bounded by collateral / (1 + bonus).
#[rule]
fn ideal_repayment_targets_102(
    e: Env,
    total_debt_wad: i128,
    weighted_collateral_wad: i128,
    hf_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
) {
    cvlr_assume!(total_debt_wad > 0);
    cvlr_assume!(total_debt_wad <= 1_000_000 * WAD);
    cvlr_assume!(weighted_collateral_wad > 0);
    cvlr_assume!(weighted_collateral_wad < total_debt_wad);
    cvlr_assume!(hf_wad > 0);
    cvlr_assume!(hf_wad < WAD);
    cvlr_assume!(base_bonus_bps > 0);
    cvlr_assume!(base_bonus_bps <= 500);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= BPS);

    let proportion_seized_wad = mul_div_half_up(&e, weighted_collateral_wad, WAD, total_debt_wad);
    let total_collateral_wad = total_debt_wad;

    let snap = crate::positions::liquidation_math::LiquidationSnapshot {
        total_debt: Wad::from(total_debt_wad),
        total_collateral: Wad::from(total_collateral_wad),
        weighted_coll: Wad::from(weighted_collateral_wad),
        proportion_seized: Wad::from(proportion_seized_wad),
        hf: Wad::from(hf_wad),
    };
    let bounds = crate::positions::liquidation_math::BonusBounds {
        base: Bps::from(base_bonus_bps),
        max: Bps::from(max_bonus_bps),
    };
    let (ideal, bonus) =
        crate::positions::liquidation_math::estimate_liquidation_amount(&e, &snap, bounds);

    cvlr_assert!(ideal.raw() > 0);
    cvlr_assert!(ideal.raw() <= total_debt_wad);

    let bonus_wad = bonus.to_wad(&e);
    let one_plus_bonus = Wad::ONE + bonus_wad;
    let max_repayable = Wad::from(total_collateral_wad).div(&e, one_plus_bonus);
    cvlr_assert!(ideal.raw() <= max_repayable.raw() + 1);
}

/// Bonus calculation is reachable for liquidatable HF (non-vacuous).
#[rule]
fn liquidation_bonus_sanity(e: Env) {
    let hf: i128 = cvlr::nondet::nondet();
    let base: i128 = cvlr::nondet::nondet();
    let max: i128 = cvlr::nondet::nondet();
    let target: i128 = cvlr::nondet::nondet();
    cvlr_assume!(hf > 0 && hf < WAD);
    cvlr_assume!(base > 0 && base <= 500);
    cvlr_assume!(max >= base && max <= BPS);
    cvlr_assume!(target > 0 && target <= 2 * WAD);

    let bonus = crate::positions::liquidation_math::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf),
        Bps::from(base),
        Bps::from(max),
        Wad::from(target),
    );
    cvlr_satisfy!(bonus.raw() > 0);
}

/// Estimate liquidation amount is reachable for liquidatable accounts (non-vacuous).
#[rule]
fn estimate_liquidation_sanity(e: Env) {
    let total_debt: i128 = cvlr::nondet::nondet();
    let weighted_col: i128 = cvlr::nondet::nondet();
    let hf: i128 = cvlr::nondet::nondet();
    cvlr_assume!(total_debt > WAD && total_debt < 1_000_000 * WAD);
    cvlr_assume!(weighted_col > 0 && weighted_col < total_debt);
    cvlr_assume!(hf > 0 && hf < WAD);

    let snap = crate::positions::liquidation_math::LiquidationSnapshot {
        total_debt: Wad::from(total_debt),
        total_collateral: Wad::from(total_debt),
        weighted_coll: Wad::from(weighted_col),
        proportion_seized: Wad::from(WAD / 2),
        hf: Wad::from(hf),
    };
    let bounds = crate::positions::liquidation_math::BonusBounds {
        base: Bps::from(500),
        max: Bps::from(1000),
    };
    let (ideal, _bonus) =
        crate::positions::liquidation_math::estimate_liquidation_amount(&e, &snap, bounds);
    cvlr_satisfy!(ideal.raw() > 0);
}
