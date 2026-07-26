//! Liquidation bonus curve and ideal-repay solver.
//!
//! Pure arithmetic over an account snapshot: reads no storage, no oracle, and
//! no `Cache`. Everything here is a function of the values passed in, so the
//! economics can be reviewed and unit-tested without standing up a ledger.
//! Pricing the account into a snapshot is `math.rs`'s job.

use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Wad};
use common::types::SpokeConfig;
use soroban_sdk::{panic_with_error, Env};

/// Pre-liq account metrics for ideal-repay and bonus math.
#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    // dimensional: debt/collateral/weighted_coll are Wad<USD>; proportion/hf are Wad<1>.
    pub total_debt: Wad,
    pub total_collateral: Wad,
    pub weighted_coll: Wad,
    pub proportion_seized: Wad,
    pub hf: Wad,
}

/// Value-weighted base bonus and protocol max ceiling for the account mix.
#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    pub base: Bps,
    pub max: Bps,
}

/// Residual bad debt: underwater and collateral USD at or below the threshold.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

/// Spoke-stamped liquidation curve (target HF, max-bonus HF, bonus factor).
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

    /// Linear bonus scale in `[0, 1]` as `hf` falls below `target`; the scale
    /// reaches 1 once `hf <= hf_for_max_bonus`. The caller guarantees
    /// `hf < target`.
    fn bonus_scale(&self, env: &Env, hf: Wad, target: Wad) -> Wad {
        let gap = target.checked_sub(env, hf);
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            gap.div(env, target.checked_sub(env, self.hf_for_max_bonus))
                .min(Wad::ONE)
        }
    }

    /// Scales a raw bonus increment by the configured factor. The default
    /// factor (1.0x) returns the increment unchanged for byte-identical output.
    fn apply_bonus_factor(&self, env: &Env, increment: i128) -> i128 {
        if self.bonus_factor == Bps::ONE {
            increment
        } else {
            self.bonus_factor.apply_to(env, increment)
        }
    }
}

/// Interpolates liquidation bonus from base to max as HF falls below target,
/// following the account's resolved liquidation curve.
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

/// Max HF-preserving bonus (BPS, floored), or `None` when no finite cap applies.
/// Cap is `hf / proportion_seized - 1`; negative means no safe partial.
/// `None` when proportion ≤ 0 or `hf >= 1`.
pub(super) fn max_hf_preserving_bonus_bps(snap: &LiquidationSnapshot) -> Option<i128> {
    let proportion = snap.proportion_seized.raw();
    if proportion <= 0 || snap.hf.raw() >= WAD {
        return None;
    }
    // hf < WAD here, so hf * BPS <= 1e22 cannot overflow. Floor division
    // rounds the cap down (stricter direction).
    Some(snap.hf.raw() * BPS / proportion - BPS)
}

/// Ideal repay + bonus: HF-scaled, HF-preserving cap, target restore, dust full-close.
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
        // HF-neutral cap when scaled bonus would ratchet HF down.
        Some(cap) if cap >= bounds.base.raw() => Bps::from(cap),
        // Base bonus shrinks HF: full close at base (`FullCloseRequired` if solvent-toxic).
        Some(_) => return (snap.total_debt, bounds.base),
    };

    let ideal = try_liquidation_at_target(env, snap, bonus, curve.target_hf).unwrap_or_else(|| {
        snap.total_collateral
            .div(env, Wad::ONE.checked_add(env, bonus.to_wad(env)))
            .min(snap.total_debt)
    });

    // Dust: sub-floor remainder → full close (no un-liquidatable dust).
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
    // dimensional: post HF = weighted collateral Wad<USD> / debt Wad<USD>.
    let one_plus_bonus = Bps::ONE.checked_add(env, bonus);

    // dimensional: Wad<1> * debt Wad<USD>, then Bps multiplier, stays Wad<USD>.
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

    // dimensional: denominator terms are Wad<1>; numerator below is Wad<USD>.
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

/// Max bonus such that effective_threshold × (1 + bonus) stays ≤ 1.
pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::from(0);
    }
    // Ceil the threshold and floor the derived max so the realized
    // effective_threshold * (1 + bonus) stays <= 1.
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
