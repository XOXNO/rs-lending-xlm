use common::constants::{MAX_LIQUIDATION_BONUS, WAD};
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionRaw, Payment, RepayEntry, SeizeEntry,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::helpers;
use crate::storage::iter_typed_positions;
use crate::utils;
use crate::validation;

/// Aggregate position metrics consumed by the liquidation pipeline. Bundling
/// these avoids repeating the same 5-value tuple across every helper.
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

pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut ControllerCache,
) -> (Wad, BonusBounds) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bounds = get_account_bonus_params(env, cache, &account.supply_positions);

    (proportion_seized, bounds)
}
pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<Payment>,
    account: &Account,
    refunds: &mut Vec<Payment>,
    cache: &mut ControllerCache,
) -> (Wad, Vec<RepayEntry>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    for i in 0..merged.len() {
        let (asset, amount) = validation::expect_invariant(env, merged.get(i));
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let position: AccountPosition = (&account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();

        let actual_debt = position
            .scaled_amount
            .mul(env, market_index.borrow_index)
            .to_asset(feed.asset_decimals);

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back((asset.clone(), excess));
            payment_amount = actual_debt;
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

    (total_repaid_usd, repaid_tokens)
}
pub(crate) fn account_dust_floors(cache: &mut ControllerCache, account: &Account) -> (i128, i128) {
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
/// Inputs to the dust-residue full-close check. `payment_ceiling_usd` is an
/// upper bound on what the liquidator has actually delivered — dust expansion
/// may never price seizure beyond this amount, otherwise the liquidator would
/// get full-close collateral for a partial payment.
#[derive(Clone, Copy)]
pub(crate) struct DustExpansionInputs<'a> {
    pub snap: &'a LiquidationSnapshot,
    pub bonus: Bps,
    pub payment_ceiling_usd: Wad,
    pub repay_usd: Wad,
}

/// Returns an updated `repay_usd` that expands to a full close when leaving
/// residue would fall below dust floors.
pub(crate) fn expand_to_full_close_on_dust_residue(
    env: &Env,
    cache: &mut ControllerCache,
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

    // Full close is only safe when the liquidator has covered the
    // entire debt — otherwise the post-state still leaves sub-floor
    // residue on at least one side. Two cases:
    //
    //   1. Payment covers total debt → expand to a real full close.
    //      Per-asset seizure capping clamps any overshoot.
    //   2. Payment is short → no partial expansion is dust-safe.
    //      Reject with `DustResidueNotAllowed`; the liquidator must
    //      either pay the full debt or pick an amount that doesn't
    //      trip the floor.
    if inputs.payment_ceiling_usd >= inputs.snap.total_debt {
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
) -> (Wad, Wad, Bps) {
    let (ideal_repayment_usd, bonus) =
        estimate_liquidation_amount(env, snap, bonus_bounds);

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);
    let seizure_multiplier = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = final_repayment_usd.mul(env, seizure_multiplier);

    (final_repayment_usd, total_seizure_usd, bonus)
}
pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment_usd: Wad,
    bonus: Bps,
    cache: &mut ControllerCache,
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

        // Split the seized RAY amount into base and bonus before computing
        // the protocol fee. Floor division keeps the base side as the lower
        // bound, so the bonus side captures any rounding remainder.
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee = asset_config.liquidation_fees.apply_to_ray(env, bonus_ray);
        let capped_amount = capped_ray.to_asset(feed.asset_decimals);
        if capped_amount <= 0 {
            continue;
        }

        seized.push_back(SeizeEntry {
            asset,
            amount: capped_amount,
            protocol_fee: protocol_fee.to_asset(feed.asset_decimals),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}
pub(crate) fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<Payment>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    while remaining_excess_usd > Wad::ZERO && !repaid_tokens.is_empty() {
        let current_index = repaid_tokens.len() - 1;
        let entry = validation::expect_invariant(env, repaid_tokens.get(current_index));

        let usd = Wad::from_raw(entry.usd_wad);

        if usd > remaining_excess_usd {
            let ratio = remaining_excess_usd.div(env, usd);
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul(env, ratio)
                .to_token(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;
            // Recompute `new_usd` from `new_amount * price`. Subtracting the
            // excess directly lets the two precision paths drift and leaves
            // the RepayEntry pair inconsistent for downstream consumers.
            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from_raw(entry.feed.price_wad));

            refunds.push_back((entry.asset.clone(), refund_amount));
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
            refunds.push_back((entry.asset, entry.amount));
            repaid_tokens.remove(current_index);
            remaining_excess_usd -= usd;
        }
    }
}

// Interpolates liquidation bonus linearly from base to max.
pub fn calculate_linear_bonus_with_target(
    env: &Env,
    hf: Wad,
    base: Bps,
    max: Bps,
    target: Wad,
) -> Bps {
    let gap_numerator = target - hf;
    if gap_numerator <= Wad::ZERO {
        return base;
    }
    let gap_wad = gap_numerator.div(env, target);

    let double_gap = gap_wad.mul(env, Wad::from_raw(2 * WAD));
    let scale = double_gap.min(Wad::ONE);

    let bonus_range = max - base;
    let bonus_increment = Wad::from_raw(bonus_range.raw()).mul(env, scale).raw();
    let bonus = Bps::from_raw(
        base.raw()
            .checked_add(bonus_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    );

    Bps::from_raw(bonus.raw().min(MAX_LIQUIDATION_BONUS))
}

// Estimates optimal debt repayment and bonus.
pub(crate) fn estimate_liquidation_amount(
    env: &Env,
    snap: &LiquidationSnapshot,
    bounds: BonusBounds,
) -> (Wad, Bps) {
    let target_primary = Wad::from_raw(1_020_000_000_000_000_000i128);
    let bonus_primary =
        calculate_linear_bonus_with_target(env, snap.hf, bounds.base, bounds.max, target_primary);
    if let Some(d) = try_liquidation_at_target(env, snap, bonus_primary, target_primary) {
        let new_hf = calculate_post_liquidation_hf(env, snap, d, bonus_primary);
        if new_hf >= Wad::ONE {
            return (d, bonus_primary);
        }
    }

    let target_fallback = Wad::from_raw(WAD + WAD / 100);
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
    let seized_weighted = Wad::from_raw(seized_weighted_raw).min(snap.weighted_coll);

    let new_weighted = snap.weighted_coll - seized_weighted;
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt - debt_to_repay
    };

    if new_debt == Wad::ZERO {
        return Wad::from_raw(i128::MAX);
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
    let denominator = target_hf - denom_term;

    if denominator <= Wad::ZERO {
        return None;
    }

    let target_debt = target_hf.mul(env, snap.total_debt);
    if target_debt <= snap.weighted_coll {
        return Some(d_max.min(snap.total_debt));
    }
    let numerator = target_debt - snap.weighted_coll;
    let d_ideal = numerator.div(env, denominator);

    Some(d_ideal.min(d_max).min(snap.total_debt))
}

// Returns collateral-value-weighted average liquidation bonus.
pub(crate) fn get_account_bonus_params(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
) -> BonusBounds {
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
            base: Bps::from_raw(0),
            max: Bps::from_raw(MAX_LIQUIDATION_BONUS),
        };
    }

    let mut weighted_bonus_sum: i128 = 0;
    for i in 0..asset_values.len() {
        let (value_raw, bonus_bps) = validation::expect_invariant(env, asset_values.get(i));
        let weight = Wad::from_raw(value_raw).div(env, total_collateral);
        weighted_bonus_sum = weighted_bonus_sum
            .checked_add(weight.mul(env, Wad::from_raw(bonus_bps)).raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    BonusBounds {
        base: Bps::from_raw(weighted_bonus_sum),
        max: Bps::from_raw(MAX_LIQUIDATION_BONUS),
    }
}
