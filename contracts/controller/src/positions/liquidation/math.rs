//! Liquidation accounting.

use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
use common::types::{
    Account, AccountPositionRaw, DebtPosition, HubAssetKey, LiquidationResult, PaymentTuple,
    RepayEntry, SeizeEntry, SpokeConfig,
};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

use crate::context::Cache;
use crate::payments as utils;
use crate::positions::HubPayment;
use crate::risk;
use crate::risk::validation;
use crate::storage::iter_typed_positions;

/// Aggregate position metrics for liquidation helpers.
#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    // dimensional: debt/collateral/weighted_coll are Wad<USD>; proportion/hf are Wad<1>.
    pub total_debt: Wad,
    pub total_collateral: Wad,
    pub weighted_coll: Wad,
    pub proportion_seized: Wad,
    pub hf: Wad,
}

/// Liquidation bonus interpolation bounds (base and protocol-max).
#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    pub base: Bps,
    pub max: Bps,
}

/// Repayment legs after close-amount, excess-refund, and dust-residue caps.
pub(crate) struct NormalizedRepaymentPlan {
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub repay_usd: Wad,
    pub bonus: Bps,
}

impl NormalizedRepaymentPlan {
    /// Panics unless the repaid legs sum to the recorded repay total.
    fn validate(&self, env: &Env) {
        if sum_repaid_usd(&self.repaid) != self.repay_usd {
            panic_with_error!(env, GenericError::InternalError);
        }
    }
}

/// Fully-priced liquidation plan. This is the handoff object from pure
/// liquidation math to stateful pool execution.
pub(crate) struct LiquidationPlan {
    pub repayment: NormalizedRepaymentPlan,
    pub seized: Vec<SeizeEntry>,
}

impl LiquidationPlan {
    /// Panics unless the repayment and every seize leg satisfy plan invariants.
    pub(crate) fn validate(&self, env: &Env) {
        self.repayment.validate(env);

        for entry in self.seized.iter() {
            if entry.amount <= 0 || entry.protocol_fee < 0 || entry.protocol_fee > entry.amount {
                panic_with_error!(env, GenericError::InternalError);
            }
        }
    }

    /// Converts the plan into the executable `LiquidationResult`.
    pub(crate) fn into_result(self) -> LiquidationResult {
        LiquidationResult {
            seized: self.seized,
            repaid: self.repayment.repaid,
            refunds: self.repayment.refunds,
            max_debt_usd: self.repayment.repay_usd.raw(),
            bonus_bps: self.repayment.bonus.raw(),
        }
    }
}

/// True when collateral is small enough for bad-debt socialization.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

/// Computes the weighted seizure proportion and the account's bonus bounds.
pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut Cache,
) -> (Wad, BonusBounds) {
    // dimensional: weighted collateral Wad<USD> / collateral Wad<USD> -> Wad<1>.
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bounds = get_account_bonus_params(
        env,
        cache,
        account.spoke_id,
        &account.supply_positions,
        proportion_seized,
    );

    (proportion_seized, bounds)
}

/// Prices each debt leg, diverting over-repayment into refunds, and returns the
/// total repaid USD with per-leg entries.
pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<HubPayment>,
    account: &Account,
    refunds: &mut Vec<PaymentTuple>,
    cache: &mut Cache,
) -> (Wad, Vec<RepayEntry>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    for (hub_asset, amount) in merged {
        let feed = cache.cached_price_for(account.spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let position: DebtPosition = (&account
            .borrow_positions
            .get(hub_asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();

        let actual_debt = debt_close_amount(
            env,
            &position,
            market_index.borrow_index,
            feed.asset_decimals,
        );

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back(PaymentTuple {
                asset: hub_asset.asset.clone(),
                amount: excess,
            });
            payment_amount = actual_debt;
        }

        let payment_usd = feed.usd_value_wad(env, payment_amount);

        total_repaid_usd += payment_usd;
        repaid_tokens.push_back(RepayEntry {
            hub_asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    (total_repaid_usd, repaid_tokens)
}

/// Prices repay legs, caps them to the max repayable USD, and returns the
/// validated repayment plan.
pub(crate) fn normalize_repayment_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    curve: &LiquidationCurve,
    cache: &mut Cache,
) -> NormalizedRepaymentPlan {
    let mut refunds = Vec::new(env);
    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, raw_payments, account, &mut refunds, cache);

    let (ideal_repayment_usd, bonus) = estimate_liquidation_amount(env, snap, bonus_bounds, curve);
    let max_debt_to_repay_usd = total_debt_payment_usd.min(ideal_repayment_usd);

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd - max_debt_to_repay_usd;
        process_excess_payment(env, &mut final_repayment_tokens, &mut refunds, excess_usd);
    }

    let repayment = NormalizedRepaymentPlan {
        repay_usd: sum_repaid_usd(&final_repayment_tokens),
        repaid: final_repayment_tokens,
        refunds,
        bonus,
    };
    repayment.validate(env);
    repayment
}

/// Returns the token amount that fully closes the debt position, rounding up.
fn debt_close_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    // dimensional: debt share/index -> Ray<Token(asset)>; to_asset_ceil returns Token(asset).
    position
        .scaled_amount
        .mul(env, borrow_index)
        .to_asset_ceil(asset_decimals)
}

/// Sums the USD value across the repaid legs.
pub(crate) fn sum_repaid_usd(repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        total += Wad::from(entry.usd_wad);
    }
    total
}

/// Distributes the bonused seizure across collateral, applying per-asset caps
/// and protocol fees.
pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment: &NormalizedRepaymentPlan,
    cache: &mut Cache,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + repayment.bonus.to_wad(env);
    // dimensional: DebtRepaid<Wad<USD>> * (1 + bonus Bps) -> Seize<Wad<USD>>.
    let total_seizure_usd = repayment.repay_usd.mul(env, one_plus_bonus);

    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        let feed = cache.cached_price_for(account.spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // dimensional: supply share/index -> Token(asset) -> Wad<USD>; share is Wad<1>.
        let actual_ray = position.scaled_amount.mul(env, market_index.supply_index);
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, feed.price);

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        // dimensional: Seize<Wad<USD>> / Price<Wad<USD/token>> -> Token Wad, then Ray.
        let seizure_amount_wad = seizure_for_asset_usd.div(env, feed.price);
        let seizure_ray = seizure_amount_wad.to_ray();

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }

        // Floor-divide so the base side is the lower bound; the bonus side
        // absorbs the rounding remainder before the protocol fee applies.
        // The fee is position-snapshotted (like bonus), so delisted collateral
        // keeps its last-stamped fee under the same `Frozen` policy as withdraw.
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee_ray = position.liquidation_fees.apply_to_ray(env, bonus_ray);
        // Full seizure uses pool half-up conversion so pool full-close succeeds.
        // Partial seizures floor and cannot exceed the computed RAY amount.
        let capped_amount = if capped_ray == actual_ray {
            capped_ray.to_asset(feed.asset_decimals)
        } else {
            capped_ray.to_asset_floor(feed.asset_decimals)
        };
        if capped_amount <= 0 {
            continue;
        }

        // Positive protocol fee has a one-unit minimum.
        // `capped_amount >= 1` here, so `fee <= amount` still holds.
        let fee_asset = protocol_fee_ray.to_asset_floor(feed.asset_decimals);
        let protocol_fee = if protocol_fee_ray > Ray::ZERO && fee_asset == 0 {
            1
        } else {
            fee_asset
        };

        seized.push_back(SeizeEntry {
            hub_asset,
            amount: capped_amount,
            protocol_fee,
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

/// Refunds over-repayment USD from the tail repay legs, floor-splitting the
/// boundary leg.
pub(crate) fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<PaymentTuple>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    let mut current_index = repaid_tokens.len();
    while remaining_excess_usd > Wad::ZERO && current_index > 0 {
        current_index -= 1;
        let entry = validation::expect_invariant(env, repaid_tokens.get(current_index));
        if entry.amount <= 0 {
            continue;
        }

        let usd = Wad::from(entry.usd_wad);
        if usd == Wad::ZERO {
            continue;
        }

        if usd > remaining_excess_usd {
            // Floor each step: the refund returned to the payer cannot exceed
            // the exact pro-rata share; sub-ulp remainder stays as repayment.
            // dimensional: excess Wad<USD> / entry Wad<USD> -> Wad<1>.
            let ratio = remaining_excess_usd.div_floor(env, usd);
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul_floor(env, ratio)
                .to_token_floor(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;
            // Recompute new_usd from new_amount * price; subtracting the excess
            // directly drifts the two precision paths and desyncs the RepayEntry pair.
            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from(entry.feed.price_wad));

            refunds.push_back(PaymentTuple {
                asset: entry.hub_asset.asset.clone(),
                amount: refund_amount,
            });
            repaid_tokens.set(
                current_index,
                RepayEntry {
                    hub_asset: entry.hub_asset,
                    amount: new_amount,
                    usd_wad: new_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back(PaymentTuple {
                asset: entry.hub_asset.asset.clone(),
                amount: entry.amount,
            });
            repaid_tokens.remove(current_index);
            remaining_excess_usd -= usd;
        }
    }
}

/// Resolved liquidation curve for an account's spoke. Spoke creation stamps
/// the default curve values, so storage always carries effective parameters.
pub(crate) struct LiquidationCurve {
    target_hf: Wad,
    hf_for_max_bonus: Wad,
    bonus_factor: Bps,
}

impl LiquidationCurve {
    /// Resolves the curve from the account's spoke.
    pub(crate) fn resolve(cache: &mut Cache, spoke_id: u32) -> Self {
        Self::from_config(&cache.spoke_config(spoke_id))
    }

    /// Builds the curve from the spoke config's stored values.
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
        let gap = target - hf;
        if target <= self.hf_for_max_bonus {
            Wad::ONE
        } else {
            gap.div(env, target - self.hf_for_max_bonus).min(Wad::ONE)
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
pub fn calculate_linear_bonus_with_target(
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

    let bonus_range = max - base;
    let bonus_increment = Wad::from(bonus_range.raw()).mul(env, scale).raw();
    let scaled_increment = curve.apply_bonus_factor(env, bonus_increment);
    Bps::from(
        base.raw()
            .checked_add(scaled_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    )
}

/// Estimates the ideal liquidation repayment and its bonus (Aave V4 model).
///
/// A single health-factor-scaled bonus is applied: the protocol max below the
/// curve's max-bonus floor, interpolated toward the base above it, monotone in
/// HF. The repayment restores HF to the curve target; when the bonus makes the
/// target unreachable it is the largest the collateral supports,
/// `collateral / (1 + bonus)`. A dust guard escalates a sub-floor debt remainder
/// to a full close. Both are capped at `total_debt`.
pub(crate) fn estimate_liquidation_amount(
    env: &Env,
    snap: &LiquidationSnapshot,
    bounds: BonusBounds,
    curve: &LiquidationCurve,
) -> (Wad, Bps) {
    let bonus = calculate_linear_bonus_with_target(
        env,
        snap.hf,
        bounds.base,
        bounds.max,
        curve,
        curve.target_hf,
    );

    let ideal = try_liquidation_at_target(env, snap, bonus, curve.target_hf).unwrap_or_else(|| {
        snap.total_collateral
            .div(env, Wad::ONE + bonus.to_wad(env))
            .min(snap.total_debt)
    });

    // Escalate a sub-floor debt remainder to a full close: the position keeps
    // either zero debt or an amount above the socialization floor, never
    // un-liquidatable dust. `ideal` is capped at `total_debt`, so the remainder
    // is non-negative.
    let remaining_debt = snap.total_debt - ideal;
    if remaining_debt > Wad::ZERO && remaining_debt < Wad::from(BAD_DEBT_USD_THRESHOLD) {
        return (snap.total_debt, bonus);
    }

    (ideal, bonus)
}

/// Returns the account's health factor after repaying `debt_to_repay` at `bonus`.
#[cfg(test)]
fn calculate_post_liquidation_hf(
    env: &Env,
    snap: &LiquidationSnapshot,
    debt_to_repay: Wad,
    bonus: Bps,
) -> Wad {
    // dimensional: post HF = weighted collateral Wad<USD> / debt Wad<USD>.
    let one_plus_bonus = Bps::ONE + bonus;

    // dimensional: Wad<1> * debt Wad<USD>, then Bps multiplier, stays Wad<USD>.
    let seized_proportion = snap.proportion_seized.mul(env, debt_to_repay);
    let seized_weighted_raw = one_plus_bonus.apply_to(env, seized_proportion.raw());
    let seized_weighted = Wad::from(seized_weighted_raw).min(snap.weighted_coll);

    let new_weighted = snap.weighted_coll - seized_weighted;
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt - debt_to_repay
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
    let one_plus_bonus = Wad::ONE + bonus_wad;

    let d_max = snap.total_collateral.div(env, one_plus_bonus);

    // dimensional: denominator terms are Wad<1>; numerator below is Wad<USD>.
    let denom_term = snap.proportion_seized.mul(env, one_plus_bonus);
    if target_hf <= denom_term {
        return None;
    }
    let denominator = target_hf - denom_term;

    let target_debt = target_hf.mul(env, snap.total_debt);
    if target_debt <= snap.weighted_coll {
        return Some(d_max.min(snap.total_debt));
    }
    let numerator = target_debt - snap.weighted_coll;
    let d_ideal = numerator.div(env, denominator);

    Some(d_ideal.min(d_max).min(snap.total_debt))
}

/// Largest liquidation bonus that keeps seizure below account collateral.
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

/// Returns base and max liquidation bonus for the account collateral mix.
pub(crate) fn get_account_bonus_params(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    proportion_seized: Wad,
) -> BonusBounds {
    let max = max_bonus_for_threshold(env, proportion_seized);

    let mut total_collateral = Wad::ZERO;
    let mut asset_values: Vec<(i128, i128)> = Vec::new(env);

    // dimensional: stores (collateral Wad<USD>.raw, bonus Bps.raw).
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let value = risk::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        total_collateral += value;
        asset_values.push_back((value.raw(), position.liquidation_bonus.raw()));
    }

    if total_collateral == Wad::ZERO {
        return BonusBounds {
            base: Bps::from(0),
            max,
        };
    }

    let mut weighted_bonus_sum: i128 = 0;
    for (value_raw, bonus_bps) in asset_values.iter() {
        let weight = Wad::from(value_raw).div(env, total_collateral);
        weighted_bonus_sum = weighted_bonus_sum
            .checked_add(weight.mul(env, Wad::from(bonus_bps)).raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    // Clamp base to the account ceiling so the bonus interpolation range
    // (max - base) stays non-negative for high-threshold accounts.
    let base = Bps::from(weighted_bonus_sum.min(max.raw()));
    BonusBounds { base, max }
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_math.rs"]
mod tests;
