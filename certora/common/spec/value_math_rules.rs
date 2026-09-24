//! Ratio lemmas: health factors and seizure splits in WAD, the Transfer-mode
//! protocol fee in RAY and token units.
//!
//! Every rule here is a statement about `crate::math`: which way a health-factor
//! division rounds, that a seizure split across two collaterals over-seizes by at
//! most one unit, and that the protocol fee stays within the whole units the
//! liquidator is paid above the repayment share.
//! No rule reads controller state or calls controller code.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::Env;

use crate::constants::{BPS, WAD};
use crate::math::fp::{Bps, Ray, Wad};
use crate::math::fp_core::{mul_div_floor, mul_div_half_up};

/// Decimals of the collateral asset in the fee model: one token unit is `1e9` RAY.
const ASSET_DECIMALS: u32 = 18;

/// Ceiling on a seizure leg in RAY, `1e10` token units at [`ASSET_DECIMALS`].
/// Keeps `seizure * WAD` inside the native `i128` path.
const MAX_SEIZURE_RAY: i128 = 10_000_000_000_000_000_000;

/// Ceiling of `max_bonus_for_threshold`, reached at a one-bps seizure proportion.
const MAX_BONUS_BPS: i128 = BPS * (BPS - 1);

struct FeeSplit {
    capped_ray: Ray,
    base_ray: Ray,
    bonus_ray: Ray,
    fee_ray: Ray,
    bumped_fee: i128,
    realised_excess: i128,
    protocol_fee: i128,
}

/// One seizure leg of `calculate_seized_collateral` in
/// `positions/liquidation/math.rs`. `paid` is the pool's gross payout in token
/// units. `base_ray` is `seizure.div_floor(one_plus_bonus.to_ray())` with the
/// common `1e9` factor taken out of both operands.
fn fee_split(
    e: &Env,
    seizure_ray: i128,
    actual_ray: i128,
    paid: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) -> FeeSplit {
    let one_plus_bonus = Wad::ONE.checked_add(e, Bps::from(bonus_bps).to_wad(e));
    let capped_ray = Ray::from(seizure_ray.min(actual_ray));
    let base_ray = Ray::from(mul_div_floor(e, seizure_ray, WAD, one_plus_bonus.raw()));
    let bonus_ray = if capped_ray > base_ray {
        capped_ray.checked_sub(e, base_ray)
    } else {
        Ray::ZERO
    };
    let fee_ray = Bps::from(liquidation_fees).apply_to_ray(e, bonus_ray);
    let fee_asset = fee_ray.to_asset_floor(e, ASSET_DECIMALS);
    let bumped_fee = if fee_ray > Ray::ZERO && fee_asset == 0 {
        1
    } else {
        fee_asset
    };
    let paid_ray = Ray::from_asset(e, paid, ASSET_DECIMALS);
    let realised_excess = if paid_ray > base_ray {
        paid_ray
            .checked_sub(e, base_ray)
            .to_asset_floor(e, ASSET_DECIMALS)
    } else {
        0
    };
    FeeSplit {
        capped_ray,
        base_ray,
        bonus_ray,
        fee_ray,
        bumped_fee,
        realised_excess,
        protocol_fee: bumped_fee.min(realised_excess),
    }
}

/// The clamped seizure floored to token units: the partial-close payout, and an
/// upper bound on the full-close payout.
fn floored_payout(e: &Env, seizure_ray: i128, actual_ray: i128) -> i128 {
    Ray::from(seizure_ray.min(actual_ray)).to_asset_floor(e, ASSET_DECIMALS)
}

fn assume_fee_domain(seizure_ray: i128, actual_ray: i128, bonus_bps: i128, fees: i128) {
    cvlr_assume!(seizure_ray > 0 && seizure_ray <= MAX_SEIZURE_RAY);
    cvlr_assume!(actual_ray > 0 && actual_ray <= MAX_SEIZURE_RAY);
    cvlr_assume!((0..=MAX_BONUS_BPS).contains(&bonus_bps));
    // validate_liquidation_fees rejects BPS itself.
    cvlr_assume!((0..BPS).contains(&fees));
}

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
    seizure_ray: i128,
    actual_ray: i128,
    paid: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    assume_fee_domain(seizure_ray, actual_ray, bonus_bps, liquidation_fees);
    cvlr_assume!((0..=floored_payout(&e, seizure_ray, actual_ray)).contains(&paid));
    let split = fee_split(
        &e,
        seizure_ray,
        actual_ray,
        paid,
        bonus_bps,
        liquidation_fees,
    );

    cvlr_assert!(split.bonus_ray <= split.capped_ray);
    cvlr_assert!(split.fee_ray <= split.bonus_ray);
    cvlr_assert!(split.protocol_fee >= 0);
    cvlr_assert!(split.protocol_fee <= split.realised_excess);
    // The one-unit minimum never exceeds the RAY fee rounded up to a whole unit.
    cvlr_assert!(split.protocol_fee <= split.fee_ray.to_asset_ceil(&e, ASSET_DECIMALS));

    // The liquidator keeps the repayment share rounded up to whole units, or
    // the whole payout when the payout does not cover it.
    let base_units = split.base_ray.to_asset_ceil(&e, ASSET_DECIMALS);
    cvlr_assert!(paid - split.protocol_fee >= paid.min(base_units));

    if liquidation_fees == 0 {
        cvlr_assert!(split.protocol_fee == 0);
    }

    // A seizure clamped at or below the repayment share is a bad-debt close:
    // no excess was realised, so no fee may be charged.
    if split.capped_ray <= split.base_ray {
        cvlr_assert!(split.protocol_fee == 0);
    }

    // Nothing clamped: the bonus is the whole seizure above the repayment share.
    if seizure_ray <= actual_ray {
        cvlr_assert!(split.bonus_ray.raw() == seizure_ray - split.base_ray.raw());
    }
}
/// A fee above the one-unit minimum, the minimum itself, the realised-excess cap
/// and a fee on a clamped seizure are all reachable inside the domain of
/// `protocol_fee_bonus_math`.
#[rule]
fn fee_lemmas_reachability(
    e: Env,
    seizure_ray: i128,
    actual_ray: i128,
    paid: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    assume_fee_domain(seizure_ray, actual_ray, bonus_bps, liquidation_fees);
    cvlr_assume!((0..=floored_payout(&e, seizure_ray, actual_ray)).contains(&paid));
    let split = fee_split(
        &e,
        seizure_ray,
        actual_ray,
        paid,
        bonus_bps,
        liquidation_fees,
    );

    cvlr_satisfy!(split.protocol_fee > 1);
    cvlr_satisfy!(split.fee_ray.to_asset_floor(&e, ASSET_DECIMALS) == 0 && split.protocol_fee == 1);
    cvlr_satisfy!(paid > 0 && split.bumped_fee > split.realised_excess);
    cvlr_satisfy!(split.capped_ray.raw() < seizure_ray && split.protocol_fee > 0);
}
/// Tightening the clamp may only reduce the fee, so no input can be driven to
/// pay more by seizing less. Both legs take the partial-close payout.
#[rule]
fn fee_is_monotone_non_increasing_in_the_clamp(
    e: Env,
    seizure_ray: i128,
    actual_lo: i128,
    actual_hi: i128,
    bonus_bps: i128,
    liquidation_fees: i128,
) {
    assume_fee_domain(seizure_ray, actual_lo, bonus_bps, liquidation_fees);
    cvlr_assume!(actual_hi >= actual_lo);
    cvlr_assume!(actual_hi <= MAX_SEIZURE_RAY);

    let fee_at = |actual: i128| {
        let paid = floored_payout(&e, seizure_ray, actual);
        fee_split(&e, seizure_ray, actual, paid, bonus_bps, liquidation_fees).protocol_fee
    };

    cvlr_assert!(fee_at(actual_lo) <= fee_at(actual_hi));
}
