use common::constants::{MAX_LIQUIDATION_BONUS, WAD};
use common::errors::GenericError;
use common::math::fp::{Bps, Ray, Wad};
use common::types::AccountPosition;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
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
    supply_positions: &Map<Address, AccountPosition>,
) -> Wad {
    let mut ltv = Wad::ZERO;
    for (asset, position) in supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        ltv += Bps::from_raw(position.loan_to_value_bps).apply_to_wad(env, value);
    }
    ltv
}

pub fn calculate_health_factor(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
    borrow_positions: &Map<Address, AccountPosition>,
) -> i128 {
    if borrow_positions.is_empty() {
        return i128::MAX; // No debt means infinite HF.
    }

    let mut weighted_collateral_total = Wad::ZERO;

    for (asset, position) in supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        weighted_collateral_total += weighted_collateral(
            env,
            value,
            Bps::from_raw(position.liquidation_threshold_bps),
        );
    }

    let mut total_borrow = Wad::ZERO;
    for (asset, position) in borrow_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_borrow += value;
    }

    if total_borrow == Wad::ZERO {
        return i128::MAX;
    }

    let w = soroban_sdk::I256::from_i128(env, weighted_collateral_total.raw());
    let wad = soroban_sdk::I256::from_i128(env, WAD);
    let tb = soroban_sdk::I256::from_i128(env, total_borrow.raw());
    let numerator = w.mul(&wad);
    let result = numerator.div(&tb);
    result.to_i128().unwrap_or(i128::MAX)
}

pub fn calculate_account_totals(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
    borrow_positions: &Map<Address, AccountPosition>,
) -> (Wad, Wad, Wad) {
    let mut total_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;

    for (asset, position) in supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_collateral += value;
        weighted_coll += weighted_collateral(
            env,
            value,
            Bps::from_raw(position.liquidation_threshold_bps),
        );
    }

    let total_debt = calculate_total_debt_wad(env, cache, borrow_positions);

    (total_collateral, total_debt, weighted_coll)
}

pub fn calculate_total_debt_wad(
    env: &Env,
    cache: &mut ControllerCache,
    borrow_positions: &Map<Address, AccountPosition>,
) -> Wad {
    let mut total_debt = Wad::ZERO;
    for (asset, position) in borrow_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
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

#[allow(clippy::too_many_arguments)]
// Estimates optimal debt repayment and bonus.
pub fn estimate_liquidation_amount(
    env: &Env,
    total_debt: Wad,
    weighted_coll: Wad,
    hf: Wad,
    base_bonus: Bps,
    max_bonus: Bps,
    proportion_seized: Wad,
    total_collateral: Wad,
) -> (Wad, Bps) {
    let target_primary = Wad::from_raw(1_020_000_000_000_000_000i128);
    let bonus_primary =
        calculate_linear_bonus_with_target(env, hf, base_bonus, max_bonus, target_primary);
    if let Some(d) = try_liquidation_at_target(
        env,
        total_debt,
        weighted_coll,
        bonus_primary,
        proportion_seized,
        total_collateral,
        target_primary,
    ) {
        let new_hf = calculate_post_liquidation_hf(
            env,
            weighted_coll,
            total_debt,
            d,
            proportion_seized,
            bonus_primary,
        );
        if new_hf >= Wad::ONE {
            return (d, bonus_primary);
        }
    }

    let target_fallback = Wad::from_raw(WAD + WAD / 100);
    let bonus_fallback =
        calculate_linear_bonus_with_target(env, hf, base_bonus, max_bonus, target_fallback);
    let fallback_result = try_liquidation_at_target(
        env,
        total_debt,
        weighted_coll,
        bonus_fallback,
        proportion_seized,
        total_collateral,
        target_fallback,
    );

    let base_bonus_wad = base_bonus.to_wad(env);
    let one_plus_base = Wad::ONE + base_bonus_wad;
    let d_max = total_collateral.div(env, one_plus_base).min(total_debt);

    let base_new_hf = calculate_post_liquidation_hf(
        env,
        weighted_coll,
        total_debt,
        d_max,
        proportion_seized,
        base_bonus,
    );

    if base_new_hf < Wad::ONE && base_new_hf < hf {
        return (d_max, base_bonus);
    }

    match fallback_result {
        Some(d) => (d, bonus_fallback),
        None => (d_max, base_bonus),
    }
}

fn calculate_post_liquidation_hf(
    env: &Env,
    weighted_coll: Wad,
    total_debt: Wad,
    debt_to_repay: Wad,
    proportion_seized: Wad,
    bonus: Bps,
) -> Wad {
    let one_plus_bonus = Bps::ONE + bonus;

    let seized_proportion = proportion_seized.mul(env, debt_to_repay);
    let seized_weighted_raw = one_plus_bonus.apply_to(env, seized_proportion.raw());
    let seized_weighted = Wad::from_raw(seized_weighted_raw).min(weighted_coll);

    let new_weighted = weighted_coll - seized_weighted;
    let new_debt = if debt_to_repay >= total_debt {
        Wad::ZERO
    } else {
        total_debt - debt_to_repay
    };

    if new_debt == Wad::ZERO {
        return Wad::from_raw(i128::MAX);
    }
    new_weighted.div(env, new_debt)
}

fn try_liquidation_at_target(
    env: &Env,
    total_debt: Wad,
    weighted_coll: Wad,
    bonus: Bps,
    proportion_seized: Wad,
    total_collateral: Wad,
    target_hf: Wad,
) -> Option<Wad> {
    let bonus_wad = bonus.to_wad(env);
    let one_plus_bonus = Wad::ONE + bonus_wad;

    let d_max = total_collateral.div(env, one_plus_bonus);

    let denom_term = proportion_seized.mul(env, one_plus_bonus);
    let denominator = target_hf - denom_term;

    if denominator <= Wad::ZERO {
        return None;
    }

    let target_debt = target_hf.mul(env, total_debt);
    if target_debt <= weighted_coll {
        return Some(d_max.min(total_debt));
    }
    let numerator = target_debt - weighted_coll;
    let d_ideal = numerator.div(env, denominator);

    Some(d_ideal.min(d_max).min(total_debt))
}

// Returns collateral-value-weighted average liquidation bonus.
pub fn get_account_bonus_params(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
) -> (Bps, Bps) {
    let mut total_collateral = Wad::ZERO;
    let mut asset_values: Vec<(i128, u32)> = Vec::new(env);

    for (asset, position) in supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_collateral += value;
        asset_values.push_back((value.raw(), position.liquidation_bonus_bps));
    }

    if total_collateral == Wad::ZERO {
        return (Bps::from_raw(0), Bps::from_raw(MAX_LIQUIDATION_BONUS));
    }

    let mut weighted_bonus_sum: i128 = 0;
    for i in 0..asset_values.len() {
        let (value_raw, bonus_bps) = validation::expect_invariant(env, asset_values.get(i));
        let weight = Wad::from_raw(value_raw).div(env, total_collateral);
        weighted_bonus_sum = weighted_bonus_sum
            .checked_add(weight.mul(env, Wad::from_raw(bonus_bps)).raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    (
        Bps::from_raw(weighted_bonus_sum),
        Bps::from_raw(MAX_LIQUIDATION_BONUS),
    )
}
