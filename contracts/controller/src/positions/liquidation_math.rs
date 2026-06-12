//! Liquidation close-amount, bonus, refund, and seizure accounting.
//!
//! Price math uses USD WAD. Pool-facing seizure and repayment entries use
//! asset-native units.

use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
use controller_interface::types::{
    Account, AccountPositionRaw, DebtPosition, Payment, PaymentTuple, RepayEntry, SeizeEntry,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::Cache;
use crate::helpers;
use crate::helpers::utils;
use crate::storage::iter_typed_positions;
use crate::validation;

/// Aggregate position metrics for liquidation helpers.
#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
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

/// True when collateral is small enough for bad-debt socialization.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut Cache,
) -> (Wad, BonusBounds) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bounds = get_account_bonus_params(env, cache, &account.supply_positions, proportion_seized);

    (proportion_seized, bounds)
}

pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<Payment>,
    account: &Account,
    refunds: &mut Vec<PaymentTuple>,
    cache: &mut Cache,
) -> (Wad, Vec<RepayEntry>, bool) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    // Full coverage is decided in token terms — the only quantization the
    // pool settles in: the deduped plan reaches every debt position (plan
    // assets must exist as positions, so length equality is set equality)
    // and every payment settles its position's full token debt.
    let mut covers_full_debt = merged.len() == account.borrow_positions.len();

    for i in 0..merged.len() {
        let (asset, amount) = validation::expect_invariant(env, merged.get(i));
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let position: DebtPosition = (&account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();

        let actual_debt = super::isolated_debt::actual_borrow_amount(
            env,
            &position,
            market_index.borrow_index,
            feed.asset_decimals,
        );

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back(PaymentTuple {
                asset: asset.clone(),
                amount: excess,
            });
            payment_amount = actual_debt;
        } else if payment_amount < actual_debt {
            covers_full_debt = false;
        }

        let payment_usd = feed.usd_value_wad(env, payment_amount);

        total_repaid_usd += payment_usd;
        repaid_tokens.push_back(RepayEntry {
            asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    (total_repaid_usd, repaid_tokens, covers_full_debt)
}

pub(crate) fn account_dust_floors(cache: &mut Cache, account: &Account) -> (i128, i128) {
    let mut min_collat: i128 = 0;
    for asset in account.supply_positions.keys() {
        let f = cache.cached_asset_config(&asset).min_collat_floor_usd.raw();
        if f > min_collat {
            min_collat = f;
        }
    }
    let mut min_debt: i128 = 0;
    for asset in account.borrow_positions.keys() {
        let f = cache.cached_asset_config(&asset).min_debt_floor_usd.raw();
        if f > min_debt {
            min_debt = f;
        }
    }
    (min_collat, min_debt)
}

/// Inputs to the dust-residue full-close check.
#[derive(Clone, Copy)]
pub(crate) struct DustExpansionInputs<'a> {
    pub snap: &'a LiquidationSnapshot,
    pub bonus: Bps,
    /// True when the payment plan settles every debt position's full token
    /// debt (computed in `calculate_repayment_amounts`).
    pub payment_covers_full_debt: bool,
    pub repay_usd: Wad,
}

/// Expands to full close only when a partial close would leave sub-floor residue.
pub(crate) fn expand_to_full_close_on_dust_residue(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    inputs: DustExpansionInputs<'_>,
) -> Wad {
    let (min_collat_floor, min_debt_floor) = account_dust_floors(cache, account);

    let one_plus_bonus = Wad::ONE + inputs.bonus.to_wad(env);
    let seizure_usd = inputs.repay_usd.mul(env, one_plus_bonus);

    let residual_debt = if inputs.repay_usd >= inputs.snap.total_debt {
        Wad::ZERO
    } else {
        inputs.snap.total_debt - inputs.repay_usd
    };
    let residual_collateral = if seizure_usd >= inputs.snap.total_collateral {
        Wad::ZERO
    } else {
        inputs.snap.total_collateral - seizure_usd
    };

    let leaves_debt_dust = residual_debt > Wad::ZERO && residual_debt.raw() < min_debt_floor;
    let leaves_collat_dust =
        residual_collateral > Wad::ZERO && residual_collateral.raw() < min_collat_floor;

    if !(leaves_debt_dust || leaves_collat_dust) {
        return inputs.repay_usd;
    }

    // Full close is dust-safe only when the payment covers the entire debt;
    // a short payment leaves sub-floor residue, so reject it (DustResidueNotAllowed)
    // rather than partial-expand. Coverage is a token-route predicate, not a
    // USD comparison: the ceiled gate valuation of debt sits wad-ulps above
    // the half-up payment route, which would make full close unreachable on
    // exact-grid token debt. Per-asset seizure capping clamps any overshoot.
    if inputs.payment_covers_full_debt {
        inputs.snap.total_debt
    } else {
        panic_with_error!(env, CollateralError::DustResidueNotAllowed);
    }
}

pub(crate) fn calculate_liquidation_amounts(
    env: &Env,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    total_payment_usd: Wad,
) -> (Wad, Bps) {
    let (ideal_repayment_usd, bonus) = estimate_liquidation_amount(env, snap, bonus_bounds);

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);

    (final_repayment_usd, bonus)
}

pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment_usd: Wad,
    bonus: Bps,
    cache: &mut Cache,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = repayment_usd.mul(env, one_plus_bonus);

    for (asset, position) in iter_typed_positions(&account.supply_positions) {
        let feed = cache.cached_price(&asset);
        if feed.price.raw() == 0 {
            continue;
        }

        let asset_config = cache.cached_asset_config(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual_ray = position.scaled_amount.mul(env, market_index.supply_index);
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, feed.price);

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

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
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee = asset_config.liquidation_fees.apply_to_ray(env, bonus_ray);
        // Full seizure must use the pool's half-up conversion: the pool
        // full-closes only when `amount >= unscale_supply(pos_scaled)`
        // (half-up), and a floored amount one unit short would leave a
        // sub-unit residue that trips the dust gate. Partial seizures floor
        // so they never exceed the computed RAY amount; fee <= amount holds
        // in both branches.
        let capped_amount = if capped_ray == actual_ray {
            capped_ray.to_asset(feed.asset_decimals)
        } else {
            capped_ray.to_asset_floor(feed.asset_decimals)
        };
        if capped_amount <= 0 {
            continue;
        }

        seized.push_back(SeizeEntry {
            asset,
            amount: capped_amount,
            protocol_fee: protocol_fee.to_asset_floor(feed.asset_decimals),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

pub(crate) fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<PaymentTuple>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    while remaining_excess_usd > Wad::ZERO && !repaid_tokens.is_empty() {
        let current_index = repaid_tokens.len() - 1;
        let entry = validation::expect_invariant(env, repaid_tokens.get(current_index));

        let usd = Wad::from(entry.usd_wad);

        if usd > remaining_excess_usd {
            // Floor every step: the refund returned to the payer never exceeds
            // the exact pro-rata share; sub-ulp remainder stays as repayment.
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
                asset: entry.asset.clone(),
                amount: refund_amount,
            });
            repaid_tokens.set(
                current_index,
                RepayEntry {
                    asset: entry.asset,
                    amount: new_amount,
                    usd_wad: new_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back(PaymentTuple {
                asset: entry.asset,
                amount: entry.amount,
            });
            repaid_tokens.remove(current_index);
            remaining_excess_usd -= usd;
        }
    }
}

/// Interpolates liquidation bonus from base to max as HF falls below target.
pub fn calculate_linear_bonus_with_target(
    env: &Env,
    hf: Wad,
    base: Bps,
    max: Bps,
    target: Wad,
) -> Bps {
    if hf >= target {
        return base;
    }
    let gap_numerator = target - hf;
    let gap_wad = gap_numerator.div(env, target);

    let double_gap = gap_wad.mul(env, Wad::from(2 * WAD));
    let scale = double_gap.min(Wad::ONE);

    let bonus_range = max - base;
    let bonus_increment = Wad::from(bonus_range.raw()).mul(env, scale).raw();
    Bps::from(
        base.raw()
            .checked_add(bonus_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    )
}

/// Estimates repayment and bonus using a 1.02 HF target, then 1.01 fallback,
/// then max-collateral seizure at base bonus without worsening account HF.
pub(crate) fn estimate_liquidation_amount(
    env: &Env,
    snap: &LiquidationSnapshot,
    bounds: BonusBounds,
) -> (Wad, Bps) {
    let target_primary = Wad::from(1_020_000_000_000_000_000i128);
    let bonus_primary =
        calculate_linear_bonus_with_target(env, snap.hf, bounds.base, bounds.max, target_primary);
    if let Some(d) = try_liquidation_at_target(env, snap, bonus_primary, target_primary) {
        let new_hf = calculate_post_liquidation_hf(env, snap, d, bonus_primary);
        if new_hf >= Wad::ONE {
            return (d, bonus_primary);
        }
    }

    let target_fallback = Wad::from(WAD + WAD / 100);
    let bonus_fallback =
        calculate_linear_bonus_with_target(env, snap.hf, bounds.base, bounds.max, target_fallback);
    let fallback_result = try_liquidation_at_target(env, snap, bonus_fallback, target_fallback);

    let base_bonus_wad = bounds.base.to_wad(env);
    let one_plus_base = Wad::ONE + base_bonus_wad;
    let d_max = snap
        .total_collateral
        .div(env, one_plus_base)
        .min(snap.total_debt);

    let base_new_hf = calculate_post_liquidation_hf(env, snap, d_max, bounds.base);

    if base_new_hf < Wad::ONE && base_new_hf < snap.hf {
        return (d_max, bounds.base);
    }

    match fallback_result {
        Some(d) => (d, bonus_fallback),
        None => (d_max, bounds.base),
    }
}

fn calculate_post_liquidation_hf(
    env: &Env,
    snap: &LiquidationSnapshot,
    debt_to_repay: Wad,
    bonus: Bps,
) -> Wad {
    let one_plus_bonus = Bps::ONE + bonus;

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
    supply_positions: &Map<Address, AccountPositionRaw>,
    proportion_seized: Wad,
) -> BonusBounds {
    let max = max_bonus_for_threshold(env, proportion_seized);

    let mut total_collateral = Wad::ZERO;
    let mut asset_values: Vec<(i128, i128)> = Vec::new(env);

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
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
    for i in 0..asset_values.len() {
        let (value_raw, bonus_bps) = validation::expect_invariant(env, asset_values.get(i));
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
