use common::constants::{MAX_LIQUIDATION_BONUS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Ray, Wad};
use common::types::AccountPositionRaw;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::positions::liquidation_math::{BonusBounds, LiquidationSnapshot};
use crate::storage::iter_typed_positions;
use crate::validation;

// USD value of a position.
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

// Liquidation-threshold-weighted collateral value.
pub fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad(env, value)
}

// LTV-weighted USD value sum of all supply positions.
pub fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
) -> Wad {
    let mut ltv = Wad::ZERO;
    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        ltv += position.loan_to_value.apply_to_wad(env, value);
    }
    ltv
}

pub fn calculate_health_factor(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, AccountPositionRaw>,
) -> Wad {
    if borrow_positions.is_empty() {
        return Wad::from_raw(i128::MAX); // No debt means infinite HF.
    }

    let mut weighted_collateral_total = Wad::ZERO;

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        weighted_collateral_total +=
            weighted_collateral(env, value, position.liquidation_threshold);
    }

    let mut total_borrow = Wad::ZERO;
    for (asset, position) in iter_typed_positions(borrow_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );

        total_borrow += value;
    }

    if total_borrow == Wad::ZERO {
        return Wad::from_raw(i128::MAX);
    }

    let w = soroban_sdk::I256::from_i128(env, weighted_collateral_total.raw());
    let wad = soroban_sdk::I256::from_i128(env, WAD);
    let tb = soroban_sdk::I256::from_i128(env, total_borrow.raw());
    let numerator = w.mul(&wad);
    let result = numerator.div(&tb);
    Wad::from_raw(result.to_i128().unwrap_or(i128::MAX))
}

pub fn calculate_account_totals(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, AccountPositionRaw>,
) -> (Wad, Wad, Wad) {
    let mut total_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        total_collateral += value;
        weighted_coll += weighted_collateral(env, value, position.liquidation_threshold);
    }

    let total_debt = calculate_total_debt_wad(env, cache, borrow_positions);

    (total_collateral, total_debt, weighted_coll)
}

pub fn calculate_total_debt_wad(
    env: &Env,
    cache: &mut ControllerCache,
    borrow_positions: &Map<Address, AccountPositionRaw>,
) -> Wad {
    let mut total_debt = Wad::ZERO;
    for (asset, position) in iter_typed_positions(borrow_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );

        total_debt += value;
    }
    total_debt
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
pub fn estimate_liquidation_amount(
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
pub fn get_account_bonus_params(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
) -> BonusBounds {
    let mut total_collateral = Wad::ZERO;
    let mut asset_values: Vec<(i128, i128)> = Vec::new(env);

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
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
