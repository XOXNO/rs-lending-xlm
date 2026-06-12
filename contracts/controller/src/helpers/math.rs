//! Health-factor, LTV, and debt aggregation over position maps.
//!
//! These helpers compute over data supplied by callers. Price and index reads
//! still go through `Cache`, so the active `OraclePolicy` remains the
//! caller's responsibility.

use common::math::fp::{Bps, Ray, Wad};
use common::types::{AccountPositionRaw, DebtPositionRaw};
use soroban_sdk::{Address, Env, Map};

use crate::cache::Cache;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// USD WAD value of a scaled position at the supplied index and price.
///
/// Half-up at every step: the neutral valuation for displays, dust floors,
/// and liquidation share proportions. Solvency gates use the directional
/// variants below instead.
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// `position_value` rounded down at every step — collateral-side gate
/// valuation, so rounding slack can never loosen LTV.
pub fn position_value_floor(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_floor(env, index);
    let actual_wad = actual.to_wad_floor();
    actual_wad.mul_floor(env, price)
}

/// `position_value` rounded up at every step — debt-side gate valuation, so
/// rounding slack can never understate what is owed.
pub fn position_value_ceil(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_ceil(env, index);
    let actual_wad = actual.to_wad_ceil();
    actual_wad.mul_ceil(env, price)
}

/// Collateral value weighted by liquidation threshold in BPS, rounded down:
/// the health-factor numerator never gains from weighting rounding.
pub fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad_floor(env, value)
}

pub fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
) -> Wad {
    cache.prefetch_market_indexes(&supply_positions.keys());

    let mut ltv = Wad::ZERO;
    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        // Floor the whole chain: borrowing capacity never rounds upward.
        let value = position_value_floor(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        ltv += position.loan_to_value.apply_to_wad_floor(env, value);
    }
    ltv
}

pub fn calculate_health_factor(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> Wad {
    if borrow_positions.is_empty() {
        return Wad::from(i128::MAX); // No debt means infinite HF.
    }

    let (_, total_borrow, weighted_collateral_total) =
        calculate_account_totals(env, cache, supply_positions, borrow_positions);

    if total_borrow == Wad::ZERO {
        return Wad::from(i128::MAX);
    }

    weighted_collateral_total.div_floor(env, total_borrow)
}

pub fn calculate_account_totals(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    _calculate_account_totals_impl(env, cache, supply_positions, borrow_positions)
}

#[cfg(not(feature = "certora"))]
fn _calculate_account_totals_impl(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    calculate_account_totals_body(env, cache, supply_positions, borrow_positions)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::calculate_account_totals_summary,
    pub(crate) fn _calculate_account_totals_impl(
        env: &Env,
        cache: &mut Cache,
        supply_positions: &Map<Address, AccountPositionRaw>,
        borrow_positions: &Map<Address, DebtPositionRaw>,
    ) -> (Wad, Wad, Wad) {
        calculate_account_totals_body(env, cache, supply_positions, borrow_positions)
    }
);

fn calculate_account_totals_body(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    // Prime the RedStone prefetch with every position's feeds and the pool
    // index prefetch with every position's markets before the per-asset
    // reads below.
    let mut priced_assets = supply_positions.keys();
    priced_assets.append(&borrow_positions.keys());
    crate::oracle::prefetch_redstone_feeds(cache, &priced_assets);
    cache.prefetch_market_indexes(&priced_assets);

    let mut total_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        // Neutral valuation for proportions and socialization checks; the
        // health-factor numerator gets the fully floored chain so no rounding
        // step can loosen the gate.
        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );
        let gate_value = position_value_floor(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        total_collateral += value;
        weighted_coll += weighted_collateral(env, gate_value, position.liquidation_threshold);
    }

    let total_debt = calculate_total_debt_wad(env, cache, borrow_positions);

    (total_collateral, total_debt, weighted_coll)
}

/// Thin Certora entry used by liquidation rules; production callers use
/// `calculate_linear_bonus_with_target` in `positions/liquidation_math`.
#[cfg(feature = "certora")]
pub fn calculate_linear_bonus(env: &Env, hf: Wad, base_bonus: Bps, max_bonus: Bps) -> Bps {
    crate::spec::summaries::calculate_linear_bonus_summary(env, hf, base_bonus, max_bonus)
}

pub fn calculate_total_debt_wad(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> Wad {
    cache.prefetch_market_indexes(&borrow_positions.keys());

    let mut total_debt = Wad::ZERO;
    for (asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        // Ceil the whole chain: owed value never rounds downward.
        let value = position_value_ceil(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );

        total_debt += value;
    }
    total_debt
}
