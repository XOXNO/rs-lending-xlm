use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Wad};
use common::math::fp_core::mul_div_ceil;
use common::types::SpokeConfig;
use soroban_sdk::{panic_with_error, Env};

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
            bonus_factor: Bps::from(i128::from(cfg.liquidation_bonus_factor_bps)),
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

/// Interpolates the base-to-max bonus increment as HF falls, then applies the
/// configured factor. HF at or above `target` receives the base bonus.
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
    let scaled_increment = curve.bonus_factor.apply_to(env, bonus_increment);
    Bps::from(
        base.raw()
            .checked_add(scaled_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    )
}

/// Returns the BPS ceiling satisfying `1 + bonus <= HF / proportion_seized`.
/// Returns `None` for nonpositive seizure proportion or HF >= 1 WAD.
pub(super) fn max_hf_preserving_bonus_bps(snap: &LiquidationSnapshot) -> Option<i128> {
    let proportion = snap.proportion_seized.raw();
    if proportion <= 0 || snap.hf.raw() >= WAD {
        return None;
    }

    // Nonnegative HF below 1e18 bounds `hf * BPS` below 1e22, within i128;
    // positive proportion prevents division by zero.
    Some(snap.hf.raw() * BPS / proportion - BPS)
}

/// Estimates WAD USD repayment and BPS bonus toward the target HF.
/// Caps the curve bonus to preserve HF; an infeasible base bonus forces a full
/// debt close. Also closes fully when a partial repayment would leave dust debt.
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

    // Below-base ceilings force a full close. Otherwise the cap only lowers
    // the curve bonus; it cannot increase the account's quoted incentive.
    let bonus = match max_hf_preserving_bonus_bps(snap) {
        None => scaled_bonus,
        Some(cap) if cap < bounds.base.raw() => return (snap.total_debt, bounds.base),
        Some(cap) => Bps::from(scaled_bonus.raw().min(cap)),
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
    let seized_weighted_raw = one_plus_bonus.apply_to(env, seized_proportion.raw());
    let seized_weighted = Wad::from(seized_weighted_raw).min(snap.weighted_collateral);

    let new_weighted = snap.weighted_collateral.checked_sub(env, seized_weighted);
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt.checked_sub(env, debt_to_repay)
    };

    if new_debt == Wad::ZERO {
        return Wad::from(i128::MAX);
    }
    // Production rounds the health factor down and saturates; the mirror must
    // agree or a test can accept a case the controller rejects.
    new_weighted.div_floor_saturating(env, new_debt)
}

/// Solves the target-HF repayment, capped by collateral backing and total debt.
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

/// Computes the BPS bonus ceiling for `(1 + bonus) * proportion_seized = 1`.
/// Ceils the proportion to BPS and clamps it to `[1, BPS]`; zero seizure returns zero.
pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::from(0);
    }

    let eff_thr_bps = mul_div_ceil(env, proportion_seized.raw(), BPS, WAD).clamp(1, BPS);
    let numerator = BPS
        .checked_mul(BPS - eff_thr_bps)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    Bps::from(numerator / eff_thr_bps)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_curve.rs"]
mod tests;
