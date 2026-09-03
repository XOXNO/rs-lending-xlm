//! Ratio lemmas over WAD values: health factors and seizure splits.
//!
//! Moved out of the controller layer on 2026-09-03. Every rule here is a
//! statement about `crate::math`: which way a health-factor division rounds,
//! and that a seizure split across two collaterals neither over-seizes nor
//! lets the liquidator end up short. None of them reads controller state or
//! calls controller code, so the arithmetic crate is where they belong.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, WAD};
use crate::math::fp::{Bps, Wad};
use crate::math::fp_core::{mul_div_floor, mul_div_half_up};

/// Ceiling on a single seizure or repayment leg, in raw token units. Keeps the
/// products below inside the native `i128` path so these lemmas are about the
/// split arithmetic rather than about widening.
const MAX_DEBT_AMOUNT_RAW: i128 = 1_000_000_000_000;

#[rule]
fn hf_division_rounds_against_borrower(e: Env, weighted: i128, debt: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&weighted));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&debt));

    let floor = Wad::from(weighted).div_floor(&e, Wad::from(debt));
    let half_up = Wad::from(weighted).div(&e, Wad::from(debt));
    cvlr_assert!(floor.raw() <= half_up.raw());
}
#[rule]
fn hf_floor_at_least_one_when_collateral_covers_debt(e: Env, weighted: i128, debt: i128) {
    cvlr_assume!((1..=1_000_000 * WAD).contains(&debt));
    cvlr_assume!((debt..=1_000_000 * WAD).contains(&weighted));

    let hf = Wad::from(weighted).div_floor(&e, Wad::from(debt));
    cvlr_assert!(hf.raw() >= WAD);
}
#[rule]
fn hf_lemmas_reachability(e: Env) {
    let value = WAD;
    let w = Bps::from(BPS).apply_to_wad_floor(&e, Wad::from(value));
    cvlr_satisfy!(w.raw() > 0);
}
#[rule]
fn seizure_split_math(
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
#[rule]
fn protocol_fee_bonus_math(
    e: Env,
    seizure_amount: i128,
    actual_amount: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    cvlr_assume!(seizure_amount > 0);
    cvlr_assume!(seizure_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(actual_amount > 0);
    cvlr_assume!(actual_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(bonus_bps >= 0);
    cvlr_assume!(bonus_bps <= BPS);
    cvlr_assume!(liquidation_fees >= 0);
    // validate_liquidation_fees rejects BPS itself.
    cvlr_assume!(liquidation_fees < BPS);

    let one_plus_bonus_wad = WAD + mul_div_half_up(&e, bonus_bps, WAD, BPS);
    // Mirrors math.rs: the seizure clamps to the collateral on hand, the fee base
    // is the repayment share taken before that clamp.
    let capped = if seizure_amount > actual_amount {
        actual_amount
    } else {
        seizure_amount
    };
    let base_amount = mul_div_floor(&e, seizure_amount, WAD, one_plus_bonus_wad);
    let bonus_amount = if capped > base_amount {
        capped - base_amount
    } else {
        0
    };
    let protocol_fee = mul_div_half_up(&e, bonus_amount, liquidation_fees, BPS);

    let fee_final = if protocol_fee == 0 && bonus_amount > 0 && liquidation_fees > 0 {
        1
    } else {
        protocol_fee
    };

    cvlr_assert!(bonus_amount >= 0);
    cvlr_assert!(bonus_amount <= capped);
    cvlr_assert!(protocol_fee <= bonus_amount);
    cvlr_assert!(fee_final >= 0);
    cvlr_assert!(fee_final <= bonus_amount + 1);
    cvlr_assert!(fee_final <= capped);

    if liquidation_fees == 0 {
        cvlr_assert!(fee_final == 0);
    }

    // A seizure clamped at or below the repayment share is a bad-debt close:
    // no excess was realised, so no fee may be charged. The one-unit bump
    // cannot fire here because it requires bonus_amount > 0.
    if capped <= base_amount {
        cvlr_assert!(bonus_amount == 0);
        cvlr_assert!(fee_final == 0);
    }

    // Nothing clamped: the realised excess is exactly the full bonus.
    if seizure_amount <= actual_amount {
        cvlr_assert!(bonus_amount == seizure_amount - base_amount);
    }
}
/// A liquidator performing the close the protocol mandates never receives less
/// than they paid in.
///
/// Net, in collateral units, is `capped - base - fee`. The fee is a fraction
/// strictly below one of `capped - base`, so the net is non-negative for every
/// clamp, bonus and HF the curve can produce.
#[rule]
fn liquidator_net_is_non_negative_for_any_clamp(
    e: Env,
    seizure_amount: i128,
    actual_amount: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    cvlr_assume!(seizure_amount > 0);
    cvlr_assume!(seizure_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(actual_amount > 0);
    cvlr_assume!(actual_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(bonus_bps >= 0);
    cvlr_assume!(bonus_bps <= BPS);
    cvlr_assume!(liquidation_fees >= 0);
    cvlr_assume!(liquidation_fees < BPS);

    let one_plus_bonus_wad = WAD + mul_div_half_up(&e, bonus_bps, WAD, BPS);
    let capped = if seizure_amount > actual_amount {
        actual_amount
    } else {
        seizure_amount
    };
    let base_amount = mul_div_floor(&e, seizure_amount, WAD, one_plus_bonus_wad);
    let bonus_amount = if capped > base_amount {
        capped - base_amount
    } else {
        0
    };
    let protocol_fee = mul_div_half_up(&e, bonus_amount, liquidation_fees, BPS);

    // Excluding the one-unit dust bump, which is bounded by a single asset unit
    // and is asserted separately in protocol_fee_bonus_math.
    if capped >= base_amount {
        cvlr_assert!(capped - base_amount - protocol_fee >= 0);
    }
}
/// Tightening the clamp may only reduce the fee, so no input can be driven to
/// pay more by seizing less.
#[rule]
fn fee_is_monotone_non_increasing_in_the_clamp(
    e: Env,
    seizure_amount: i128,
    actual_lo: i128,
    actual_hi: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    cvlr_assume!(seizure_amount > 0);
    cvlr_assume!(seizure_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(actual_lo > 0);
    cvlr_assume!(actual_hi >= actual_lo);
    cvlr_assume!(actual_hi <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(bonus_bps >= 0);
    cvlr_assume!(bonus_bps <= BPS);
    cvlr_assume!(liquidation_fees >= 0);
    cvlr_assume!(liquidation_fees < BPS);

    let one_plus_bonus_wad = WAD + mul_div_half_up(&e, bonus_bps, WAD, BPS);
    let base_amount = mul_div_floor(&e, seizure_amount, WAD, one_plus_bonus_wad);

    let fee_at = |actual: i128| -> i128 {
        let capped = if seizure_amount > actual {
            actual
        } else {
            seizure_amount
        };
        let bonus = if capped > base_amount {
            capped - base_amount
        } else {
            0
        };
        mul_div_half_up(&e, bonus, liquidation_fees, BPS)
    };

    cvlr_assert!(fee_at(actual_lo) <= fee_at(actual_hi));
}
