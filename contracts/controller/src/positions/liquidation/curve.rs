//! Liquidation bonus curve and target-health-factor math: derives the bonus applied
//! to a liquidation from an account's health factor, and solves for the debt amount
//! that brings an account back to its spoke's target health factor.

use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Wad};
use common::types::SpokeConfig;
use soroban_sdk::{panic_with_error, Env};

/// Snapshot of an account's liquidation-relevant state, used as input to the
/// liquidation curve and bonus calculations.
#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    pub total_debt: Wad,
    pub total_collateral: Wad,
    /// Collateral value weighted by liquidation threshold.
    pub weighted_coll: Wad,
    /// Weighted collateral value consumed per unit of debt repaid, before the
    /// liquidation bonus is applied.
    pub proportion_seized: Wad,
    /// Current health factor.
    pub hf: Wad,
}

/// Lower and upper bounds, in basis points, for the liquidation bonus applied across
/// the curve.
#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    /// Bonus applied when the health factor is at or above the target.
    pub base: Bps,
    /// Bonus applied at the health factor associated with the maximum bonus.
    pub max: Bps,
}

/// Returns true when total debt exceeds total collateral and total collateral is at
/// or below the bad-debt USD threshold.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

/// Liquidation bonus curve parameters derived from a spoke's configuration.
pub(crate) struct LiquidationCurve {
    pub(super) target_hf: Wad,
    hf_for_max_bonus: Wad,
    bonus_factor: Bps,
}

impl LiquidationCurve {
    /// Builds a `LiquidationCurve` from the spoke's configured target health factor,
    /// health factor for maximum bonus, and liquidation bonus factor.
    pub(crate) fn from_config(cfg: &SpokeConfig) -> Self {
        Self {
            target_hf: Wad::from(cfg.liquidation_target_hf_wad),
            hf_for_max_bonus: Wad::from(cfg.hf_for_max_bonus_wad),
            bonus_factor: Bps::from(i128::from(cfg.liquidation_bonus_factor_bps)),
        }
    }

    /// Computes how far `hf` has fallen below `target` relative to
    /// `hf_for_max_bonus`, as a value in `[0, Wad::ONE]`. Returns `Wad::ONE` when
    /// `target` is at or below `hf_for_max_bonus`.
    fn bonus_scale(&self, env: &Env, hf: Wad, target: Wad) -> Wad {
        let gap = target.checked_sub(env, hf);
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            gap.div(env, target.checked_sub(env, self.hf_for_max_bonus))
                .min(Wad::ONE)
        }
    }

    /// Scales `increment` by the curve's bonus factor, or returns it unchanged when
    /// the factor is `Bps::ONE`.
    fn apply_bonus_factor(&self, env: &Env, increment: i128) -> i128 {
        if self.bonus_factor == Bps::ONE {
            increment
        } else {
            self.bonus_factor.apply_to(env, increment)
        }
    }
}

/// Linearly interpolates the liquidation bonus between `base` and `max` based on how
/// far `hf` has fallen below `target`, then scales the result by the curve's bonus
/// factor. Returns `base` when `hf` is at or above `target`. Panics if the resulting
/// bonus overflows.
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

/// Computes the highest bonus, in basis points, for which
/// `proportion_seized * (1 + bonus)` does not exceed `hf`. Returns `None` when
/// `proportion_seized` is non-positive or `hf` is already at or above `Wad::ONE`.
pub(super) fn max_hf_preserving_bonus_bps(snap: &LiquidationSnapshot) -> Option<i128> {
    let proportion = snap.proportion_seized.raw();
    if proportion <= 0 || snap.hf.raw() >= WAD {
        return None;
    }

    Some(snap.hf.raw() * BPS / proportion - BPS)
}

/// Determines the debt amount to repay and the bonus to apply for a liquidation that
/// brings the account's health factor toward `curve.target_hf`. Caps the bonus so the
/// liquidation formula stays solvable. Liquidates the full debt at the base bonus when
/// the HF-preserving cap falls below the base bonus; liquidates the full debt at the
/// resolved (possibly capped) bonus when a partial repay would leave remaining debt
/// below the bad-debt USD threshold.
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
/// Test helper that computes the account's health factor after repaying
/// `debt_to_repay` at the given `bonus`. Returns `Wad::from(i128::MAX)` when the
/// repayment clears all debt.
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

/// Solves for the debt amount that brings the account's health factor to
/// `target_hf` given `bonus`, capped by the collateral available and the account's
/// total debt. Returns `None` when the target is unreachable because the
/// seized-collateral term would meet or exceed `target_hf`.
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

/// Computes the maximum bonus, in basis points, for which seizing
/// `proportion_seized` of collateral per unit of debt does not exceed the total
/// collateral available. Returns zero when `proportion_seized` is non-positive.
/// Panics on overflow while computing the numerator.
pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::from(0);
    }

    // Ceil(proportion * BPS / WAD), clamped into [1, BPS].
    let eff_thr_bps =
        common::math::fp_core::mul_div_ceil(env, proportion_seized.raw(), BPS, WAD).clamp(1, BPS);
    let numerator = BPS
        .checked_mul(BPS - eff_thr_bps)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    Bps::from(numerator / eff_thr_bps)
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_curve.rs"]
mod tests;
