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

/// Returns whether an account's residual position is eligible for dust-threshold bad-debt
/// socialization: debt exceeds collateral and collateral is at or below
/// `BAD_DEBT_USD_THRESHOLD`.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

pub(crate) struct LiquidationCurve {
    pub(super) target_hf: Wad,
    hf_for_max_bonus: Wad,
    bonus_factor: Bps,
}

impl LiquidationCurve {
    /// Builds a `LiquidationCurve` from a spoke's target health factor, max-bonus health-factor
    /// threshold, and bonus factor.
    pub(crate) fn from_config(cfg: &SpokeConfig) -> Self {
        Self {
            target_hf: Wad::from(cfg.liquidation_target_hf_wad),
            hf_for_max_bonus: Wad::from(cfg.hf_for_max_bonus_wad),
            bonus_factor: Bps::from(i128::from(cfg.liquidation_bonus_factor_bps)),
        }
    }

    /// Returns the fraction (0 to 1 WAD) of the bonus range to apply for `hf`, ramping linearly
    /// from 0 at `target` to 1 at `hf_for_max_bonus` and staying at 1 below that threshold;
    /// returns 1 outright when `target` is at or below `hf_for_max_bonus`.
    fn bonus_scale(&self, env: &Env, hf: Wad, target: Wad) -> Wad {
        let gap = target.checked_sub(env, hf);
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            gap.div(env, target.checked_sub(env, self.hf_for_max_bonus))
                .min(Wad::ONE)
        }
    }

    /// Applies the curve's configured bonus factor to `increment`, returning it unchanged when
    /// the factor is exactly one (100%).
    fn apply_bonus_factor(&self, env: &Env, increment: i128) -> i128 {
        if self.bonus_factor == Bps::ONE {
            increment
        } else {
            self.bonus_factor.apply_to(env, increment)
        }
    }
}

/// Scales the liquidation bonus above `base` toward `max` as `hf` falls from `target` to the
/// curve's max-bonus threshold (capped once `hf` reaches that threshold), then applies the
/// curve's bonus factor to the resulting increment. Returns `base` unchanged once `hf` is at or
/// above `target`.
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

/// Computes the bonus ceiling, in basis points, implied by
/// `1 + bonus <= health_factor / proportion_seized` — the largest bonus the account's current
/// health factor can support before the seizure math becomes infeasible. Returns `None` when
/// nothing is being seized or the account is already at or above one WAD health factor.
pub(super) fn max_hf_preserving_bonus_bps(snap: &LiquidationSnapshot) -> Option<i128> {
    let proportion = snap.proportion_seized.raw();
    if proportion <= 0 || snap.hf.raw() >= WAD {
        return None;
    }

    Some(snap.hf.raw() * BPS / proportion - BPS)
}

/// Computes the ideal debt-repayment amount and bonus for closing `snap` toward
/// `curve.target_hf`. Starts from the linear bonus curve, clamps it to the ceiling from
/// `max_hf_preserving_bonus_bps` — falling back to a full-debt close at the base bonus if even
/// that rate would breach the ceiling — then solves the target-health-factor equation via
/// `try_liquidation_at_target`. Promotes the result to a full close when the leftover debt after
/// a partial repayment would fall below the bad-debt dust threshold.
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

/// Solves for the debt repayment that would bring `snap`'s post-liquidation health factor to
/// exactly `target_hf` at the given `bonus`, using the linear relationship between debt,
/// weighted collateral, and seized value. Returns `None` when `target_hf` is at or below the
/// seizure's weighted rate (`proportion_seized * (1 + bonus)`), and otherwise caps the result at
/// both the collateral-backed maximum and `snap.total_debt`.
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

/// Computes the basis-point bonus solving `(1 + bonus) * proportion_seized = 1` — the same
/// ceiling `max_hf_preserving_bonus_bps` would produce at a health factor of exactly one WAD —
/// from `proportion_seized` expressed in basis points (ceil-rounded, clamped to `[1, BPS]`).
/// Returns zero when nothing is being seized.
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
