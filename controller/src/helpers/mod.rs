use common::constants::{MAX_LIQUIDATION_BONUS, WAD};
use common::fp::{Bps, Ray, Wad};
use common::types::AccountPosition;
use soroban_sdk::{Address, Env, Map, Vec};

#[cfg(test)]
pub mod testutils;

use crate::cache::ControllerCache;

// ---------------------------------------------------------------------------
// Position value helpers (used by health factor, liquidation, views)
// ---------------------------------------------------------------------------

pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

pub fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad(env, value)
}

pub fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
) -> Wad {
    let mut ltv = Wad::ZERO;
    for position in supply_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        ltv = ltv + Bps::from_raw(position.loan_to_value_bps).apply_to_wad(env, value);
    }
    ltv
}

// ---------------------------------------------------------------------------
// Health factor calculation
// ---------------------------------------------------------------------------

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

    // Sum weighted collateral.
    for position in supply_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);
        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        weighted_collateral_total = weighted_collateral_total
            + weighted_collateral(
                env,
                value,
                Bps::from_raw(position.liquidation_threshold_bps),
            );
    }

    // Sum borrow values.
    let mut total_borrow = Wad::ZERO;
    for position in borrow_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);
        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_borrow = total_borrow + value;
    }

    if total_borrow == Wad::ZERO {
        return i128::MAX;
    }

    // Compute `weighted * WAD / total_borrow` in I256 and clamp overflow to
    // `i128::MAX`. With high-decimal borrow tokens, dust debt, and large
    // collateral the numerator can exceed i128; treating overflow as infinite
    // HF keeps the account usable instead of locking it behind a panic.
    let w = soroban_sdk::I256::from_i128(env, weighted_collateral_total.raw());
    let wad = soroban_sdk::I256::from_i128(env, WAD);
    let tb = soroban_sdk::I256::from_i128(env, total_borrow.raw());
    let numerator = w.mul(&wad);
    let result = numerator.div(&tb);
    result.to_i128().unwrap_or(i128::MAX)
}

#[cfg(feature = "certora")]
pub fn calculate_health_factor_for(
    env: &Env,
    cache: &mut ControllerCache,
    account_id: u64,
) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    )
}

// ---------------------------------------------------------------------------
// Account totals (extracted from liquidation -- shared with views)
// ---------------------------------------------------------------------------

pub fn calculate_account_totals(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
    borrow_positions: &Map<Address, AccountPosition>,
) -> (Wad, Wad, Wad) {
    let mut total_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;

    for position in supply_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_collateral = total_collateral + value;
        weighted_coll = weighted_coll
            + weighted_collateral(
                env,
                value,
                Bps::from_raw(position.liquidation_threshold_bps),
            );
    }

    let mut total_debt = Wad::ZERO;
    for position in borrow_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_debt = total_debt + value;
    }

    (total_collateral, total_debt, weighted_coll)
}

// ---------------------------------------------------------------------------
// Liquidation math helpers
// ---------------------------------------------------------------------------

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
    let bonus = Bps::from_raw(base.raw() + bonus_increment);

    Bps::from_raw(bonus.raw().min(MAX_LIQUIDATION_BONUS))
}

#[cfg(feature = "certora")]
pub fn calculate_linear_bonus(env: &Env, hf: Wad, base_bonus: Bps, max_bonus: Bps) -> Bps {
    let target_hf = Wad::from_raw(1_020_000_000_000_000_000);
    calculate_linear_bonus_with_target(env, hf, base_bonus, max_bonus, target_hf)
}

#[allow(clippy::too_many_arguments)]
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
    let target_primary = Wad::from_raw(1_020_000_000_000_000_000);
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

    // 1.01 * WAD — a slight-overshoot HF target used as fallback when the
    // first attempt cannot restore HF to exactly 1.0 under the bonus curve.
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

    // Unrecoverable-position path: even the softest target leaves HF below
    // 1.0, so apply the base bonus against the maximum collateral-backed
    // repayment.
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

pub fn get_account_bonus_params(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPosition>,
) -> (Bps, Bps) {
    let mut total_collateral = Wad::ZERO;
    // Store (value_wad_raw, bonus_bps) as raw i128: Soroban Vec cannot hold Wad.
    let mut asset_values: Vec<(i128, i128)> = Vec::new(env);

    for position in supply_positions.values() {
        let feed = cache.cached_price(&position.asset);
        let market_index = cache.cached_market_index(&position.asset);

        let value = position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );

        total_collateral = total_collateral + value;
        asset_values.push_back((value.raw(), position.liquidation_bonus_bps));
    }

    if total_collateral == Wad::ZERO {
        return (Bps::from_raw(0), Bps::from_raw(MAX_LIQUIDATION_BONUS));
    }

    let mut weighted_bonus_sum: i128 = 0;
    for i in 0..asset_values.len() {
        let (value_raw, bonus_bps) = asset_values.get(i).unwrap();
        let weight = Wad::from_raw(value_raw).div(env, total_collateral);
        weighted_bonus_sum += weight.mul(env, Wad::from_raw(bonus_bps)).raw();
    }

    (
        Bps::from_raw(weighted_bonus_sum),
        Bps::from_raw(MAX_LIQUIDATION_BONUS),
    )
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{Address, Env, Map};

    #[test]
    fn test_calculate_health_factor_returns_max_without_borrows() {
        let env = Env::default();
        let mut cache = ControllerCache::new(&env, true);
        let supply_positions: Map<Address, AccountPosition> = Map::new(&env);
        let borrow_positions: Map<Address, AccountPosition> = Map::new(&env);

        assert_eq!(
            calculate_health_factor(&env, &mut cache, &supply_positions, &borrow_positions),
            i128::MAX
        );
    }

    #[test]
    fn test_calculate_linear_bonus_with_target_returns_base_bonus_when_gap_is_non_positive() {
        let env = Env::default();

        assert_eq!(
            calculate_linear_bonus_with_target(
                &env,
                Wad::from_raw(11 * WAD / 10),
                Bps::from_raw(500),
                Bps::from_raw(1_500),
                Wad::from_raw(WAD),
            ),
            Bps::from_raw(500)
        );
    }

    #[test]
    fn test_estimate_liquidation_amount_falls_back_to_base_bonus_when_target_is_unreachable() {
        let env = Env::default();
        let total_debt = Wad::from_raw(100 * WAD);
        let weighted_coll = Wad::from_raw(200 * WAD);
        let proportion_seized = Wad::ONE;
        let total_collateral = Wad::from_raw(100 * WAD);
        let base_bonus = Bps::from_raw(500);

        let (debt_to_repay, bonus) = estimate_liquidation_amount(
            &env,
            total_debt,
            weighted_coll,
            Wad::from_raw(9 * WAD / 10),
            base_bonus,
            Bps::from_raw(1_500),
            proportion_seized,
            total_collateral,
        );

        let one_plus_base = Wad::ONE + base_bonus.to_wad(&env);
        let expected_d_max = total_collateral.div(&env, one_plus_base);

        assert_eq!(bonus, base_bonus);
        assert_eq!(debt_to_repay, expected_d_max);
    }

    #[test]
    fn test_try_liquidation_at_target_caps_at_max_repay_when_target_is_already_met() {
        let env = Env::default();
        let total_debt = Wad::from_raw(100 * WAD);
        let weighted_coll = Wad::from_raw(120 * WAD);
        let bonus = Bps::from_raw(500);
        let total_collateral = Wad::from_raw(150 * WAD);
        let one_plus_bonus = Wad::ONE + bonus.to_wad(&env);
        let expected_d_max = total_collateral.div(&env, one_plus_bonus).min(total_debt);

        assert_eq!(
            try_liquidation_at_target(
                &env,
                total_debt,
                weighted_coll,
                bonus,
                Wad::from_raw(WAD / 2),
                total_collateral,
                Wad::from_raw(1_020_000_000_000_000_000),
            ),
            Some(expected_d_max)
        );
    }

    #[test]
    fn test_get_account_bonus_params_returns_protocol_max_for_empty_supply() {
        let env = Env::default();
        let mut cache = ControllerCache::new(&env, true);
        let supply_positions: Map<Address, AccountPosition> = Map::new(&env);

        assert_eq!(
            get_account_bonus_params(&env, &mut cache, &supply_positions),
            (Bps::from_raw(0), Bps::from_raw(MAX_LIQUIDATION_BONUS))
        );
    }
}
