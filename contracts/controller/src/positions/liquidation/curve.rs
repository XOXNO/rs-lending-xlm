use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Wad};
use common::types::SpokeConfig;
use soroban_sdk::{panic_with_error, Env};

#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    pub total_debt: Wad,
    pub total_collateral: Wad,
    pub weighted_coll: Wad,
    pub proportion_seized: Wad,
    pub hf: Wad,
}

#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    pub base: Bps,
    pub max: Bps,
}

pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

pub(crate) struct LiquidationCurve {
    pub(super) target_hf: Wad,
    hf_for_max_bonus: Wad,
    bonus_factor: Bps,
}

impl LiquidationCurve {
    pub(crate) fn from_config(cfg: &SpokeConfig) -> Self {
        Self {
            target_hf: Wad::from(cfg.liquidation_target_hf_wad),
            hf_for_max_bonus: Wad::from(cfg.hf_for_max_bonus_wad),
            bonus_factor: Bps::from(i128::from(cfg.liquidation_bonus_factor_bps)),
        }
    }

    fn bonus_scale(&self, env: &Env, hf: Wad, target: Wad) -> Wad {
        let gap = target.checked_sub(env, hf);
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            gap.div(env, target.checked_sub(env, self.hf_for_max_bonus))
                .min(Wad::ONE)
        }
    }

    fn apply_bonus_factor(&self, env: &Env, increment: i128) -> i128 {
        if self.bonus_factor == Bps::ONE {
            increment
        } else {
            self.bonus_factor.apply_to(env, increment)
        }
    }
}

pub(crate) fn calculate_linear_bonus_with_target(
    env: &Env,
    hf: Wad,
    base: Bps,
    max: Bps,
    curve: &LiquidationCurve,
    target: Wad,
) -> Bps {
    if hf >= target {
        return base;
    }
    let scale = curve.bonus_scale(env, hf, target);

    let bonus_range = max.checked_sub(env, base);
    let bonus_increment = Wad::from(bonus_range.raw()).mul(env, scale).raw();
    let scaled_increment = curve.apply_bonus_factor(env, bonus_increment);
    Bps::from(
        base.raw()
            .checked_add(scaled_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    )
}

pub(super) fn max_hf_preserving_bonus_bps(snap: &LiquidationSnapshot) -> Option<i128> {
    let proportion = snap.proportion_seized.raw();
    if proportion <= 0 || snap.hf.raw() >= WAD {
        return None;
    }

    Some(snap.hf.raw() * BPS / proportion - BPS)
}

pub(crate) fn estimate_liquidation_amount(
    env: &Env,
    snap: &LiquidationSnapshot,
    bounds: BonusBounds,
    curve: &LiquidationCurve,
) -> (Wad, Bps) {
    let scaled_bonus = calculate_linear_bonus_with_target(
        env,
        snap.hf,
        bounds.base,
        bounds.max,
        curve,
        curve.target_hf,
    );

    let bonus = match max_hf_preserving_bonus_bps(snap) {
        None => scaled_bonus,
        Some(cap) if scaled_bonus.raw() <= cap => scaled_bonus,

        Some(cap) if cap >= bounds.base.raw() => Bps::from(cap),

        Some(_) => return (snap.total_debt, bounds.base),
    };

    let ideal = try_liquidation_at_target(env, snap, bonus, curve.target_hf).unwrap_or_else(|| {
        snap.total_collateral
            .div(env, Wad::ONE.checked_add(env, bonus.to_wad(env)))
            .min(snap.total_debt)
    });

    let remaining_debt = snap.total_debt.checked_sub(env, ideal);
    if remaining_debt > Wad::ZERO && remaining_debt < Wad::from(BAD_DEBT_USD_THRESHOLD) {
        return (snap.total_debt, bonus);
    }

    (ideal, bonus)
}

#[cfg(test)]
pub(super) fn calculate_post_liquidation_hf(
    env: &Env,
    snap: &LiquidationSnapshot,
    debt_to_repay: Wad,
    bonus: Bps,
) -> Wad {
    let one_plus_bonus = Bps::ONE.checked_add(env, bonus);

    let seized_proportion = snap.proportion_seized.mul(env, debt_to_repay);
    let seized_weighted_raw = one_plus_bonus.apply_to(env, seized_proportion.raw());
    let seized_weighted = Wad::from(seized_weighted_raw).min(snap.weighted_coll);

    let new_weighted = snap.weighted_coll.checked_sub(env, seized_weighted);
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt.checked_sub(env, debt_to_repay)
    };

    if new_debt == Wad::ZERO {
        return Wad::from(i128::MAX);
    }
    new_weighted.div(env, new_debt)
}

fn try_liquidation_at_target(
    env: &Env,
    snap: &LiquidationSnapshot,
    bonus: Bps,
    target_hf: Wad,
) -> Option<Wad> {
    let bonus_wad = bonus.to_wad(env);
    let one_plus_bonus = Wad::ONE.checked_add(env, bonus_wad);

    let d_max = snap.total_collateral.div(env, one_plus_bonus);

    let denom_term = snap.proportion_seized.mul(env, one_plus_bonus);
    if target_hf <= denom_term {
        return None;
    }
    let denominator = target_hf.checked_sub(env, denom_term);

    let target_debt = target_hf.mul(env, snap.total_debt);
    if target_debt <= snap.weighted_coll {
        return Some(d_max.min(snap.total_debt));
    }
    let numerator = target_debt.checked_sub(env, snap.weighted_coll);
    let d_ideal = numerator.div(env, denominator);

    Some(d_ideal.min(d_max).min(snap.total_debt))
}

pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::from(0);
    }

    let scaled = proportion_seized
        .raw()
        .checked_mul(BPS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let eff_thr_bps = ((scaled + (WAD - 1)) / WAD).clamp(1, BPS);
    let numerator = BPS
        .checked_mul(BPS - eff_thr_bps)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    Bps::from(numerator / eff_thr_bps)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_curve.rs"]
mod tests;
