use crate::constants::BAD_DEBT_USD_THRESHOLD;
use common::math::fp::{Bps, Wad};
use common::types::SpokeConfig;
use soroban_sdk::Env;

#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    pub total_debt: Wad,
    pub total_collateral: Wad,
    pub weighted_collateral: Wad,
    pub proportion_seized: Wad,
    pub hf: Wad,
}

#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    pub base: Bps,
    pub max: Bps,
}

/// Result of the signed bonus ceiling that preserves the health factor. The type avoids
/// representing a negative rate as a raw integer. `BelowZero` means even a zero bonus cannot
/// preserve the health factor. `NonNegative` carries the usable BPS ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HfPreservingBonusCap {
    BelowZero,
    NonNegative(Bps),
}

impl HfPreservingBonusCap {
    #[cfg(test)]
    pub(super) fn clamped(self) -> Bps {
        match self {
            Self::BelowZero => Bps::ZERO,
            Self::NonNegative(cap) => cap,
        }
    }
}

/// Admits socialization when debt exceeds collateral and collateral is at or
/// below `BAD_DEBT_USD_THRESHOLD` (WAD USD).
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
            bonus_factor: Bps::from(cfg.liquidation_bonus_factor_bps),
        }
    }

    /// Ramps from zero at `target` to one WAD at the max-bonus threshold.
    /// Degenerate or reversed threshold ranges return one WAD.
    fn bonus_scale(&self, env: &Env, hf: Wad, target: Wad) -> Wad {
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            target
                .checked_sub(env, hf)
                .div(env, target.checked_sub(env, self.hf_for_max_bonus))
                .min(Wad::ONE)
        }
    }
}

/// Interpolates the base-to-max bonus increment as the health factor falls, then
/// applies the configured factor. A health factor at or above `target` receives the
/// base bonus.
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

    let bonus_increment = max.checked_sub(env, base).mul_wad(env, scale);
    base.checked_add(env, bonus_increment.mul(env, curve.bonus_factor))
}

/// Returns the BPS ceiling that satisfies `1 + bonus <= HF / proportion_seized`. `BelowZero`
/// records that the mathematical ceiling is negative without leaving typed fixed-point arithmetic.
/// Returns `None` for nonpositive seizure proportion or a health factor at or above 1 WAD.
pub(super) fn max_hf_preserving_bonus_bps(
    env: &Env,
    snap: &LiquidationSnapshot,
) -> Option<HfPreservingBonusCap> {
    if snap.proportion_seized <= Wad::ZERO || snap.hf >= Wad::ONE {
        return None;
    }

    let hf_over_seizure = Bps::from_fraction_floor(env, snap.hf, snap.proportion_seized);
    Some(if hf_over_seizure < Bps::ONE {
        HfPreservingBonusCap::BelowZero
    } else {
        HfPreservingBonusCap::NonNegative(hf_over_seizure.checked_sub(env, Bps::ONE))
    })
}

/// Estimates WAD USD repayment and BPS bonus toward the target health factor. Caps the curve bonus
/// to preserve the health factor. With a positive seizure proportion, collateral below debt quotes
/// the collateral-backed repayment `min(D, floor(C / (1 + base)))` at the base bonus. Otherwise a
/// cap below base quotes the full debt at the cap, floored at zero. A target-health-factor quote
/// that would leave positive debt below `BAD_DEBT_USD_THRESHOLD` becomes the full debt.
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

    let bonus = match max_hf_preserving_bonus_bps(env, snap) {
        None => scaled_bonus,
        Some(_) if snap.total_collateral < snap.total_debt => {
            let one_plus_base = Wad::ONE.checked_add(env, bounds.base.to_wad(env));
            let backed = snap.total_collateral.div_floor(env, one_plus_base);
            return (backed.min(snap.total_debt), bounds.base);
        }
        Some(HfPreservingBonusCap::BelowZero) => {
            return (snap.total_debt, Bps::ZERO);
        }
        Some(HfPreservingBonusCap::NonNegative(cap)) if cap < bounds.base => {
            return (snap.total_debt, cap);
        }
        Some(HfPreservingBonusCap::NonNegative(cap)) => scaled_bonus.min(cap),
    };

    let ideal = liquidation_at_target(env, snap, bonus, curve.target_hf);

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
    let seized_weighted = one_plus_bonus
        .apply_to_wad(env, seized_proportion)
        .min(snap.weighted_collateral);

    let new_weighted = snap.weighted_collateral.checked_sub(env, seized_weighted);
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt.checked_sub(env, debt_to_repay)
    };

    if new_debt == Wad::ZERO {
        return Wad::from(i128::MAX);
    }
    // Production rounds the health factor down and saturates. The mirror must agree, or a test can
    // accept a case the controller rejects.
    new_weighted.div_floor_saturating(env, new_debt)
}

/// Solves the target-health-factor repayment, capped by collateral backing and total debt.
/// If seizure's weighted rate meets the target, or weighted collateral already
/// covers target debt, returns the capped maximum directly.
fn liquidation_at_target(env: &Env, snap: &LiquidationSnapshot, bonus: Bps, target_hf: Wad) -> Wad {
    let one_plus_bonus = Wad::ONE.checked_add(env, bonus.to_wad(env));
    let d_max = snap.total_collateral.div(env, one_plus_bonus);
    let denom_term = snap.proportion_seized.mul(env, one_plus_bonus);
    let target_debt = target_hf.mul(env, snap.total_debt);

    if target_hf <= denom_term || target_debt <= snap.weighted_collateral {
        return d_max.min(snap.total_debt);
    }

    // Both subtractions are positive by the branch above.
    let numerator = target_debt.checked_sub(env, snap.weighted_collateral);
    let denominator = target_hf.checked_sub(env, denom_term);
    numerator
        .div(env, denominator)
        .min(d_max)
        .min(snap.total_debt)
}

/// Computes the BPS bonus ceiling for `(1 + bonus) * proportion_seized = 1`. Ceils the proportion
/// to BPS and clamps it to `[1, BPS]`. Zero seizure returns zero.
pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::ZERO;
    }

    let effective_threshold =
        Bps::from_fraction_ceil(env, proportion_seized, Wad::ONE).clamp(Bps::from(1i128), Bps::ONE);
    Bps::ONE
        .checked_sub(env, effective_threshold)
        .div_floor(env, effective_threshold)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_curve.rs"]
mod tests;
