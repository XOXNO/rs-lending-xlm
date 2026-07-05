//! Health-factor, LTV, and debt aggregation.

use common::math::fp::{Bps, Ray, Wad};
use common::types::{AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use crate::context::Cache;
use crate::oracle;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// Token addresses underlying position keys.
pub(crate) fn position_assets(env: &Env, keys: &Vec<HubAssetKey>) -> Vec<Address> {
    let mut assets: Vec<Address> = Vec::new(env);
    for key in keys.iter() {
        assets.push_back(key.asset);
    }
    assets
}

/// Neutral USD WAD value of a scaled position.
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    // dimensional: Ray<Share> * Ray<Index> -> Ray<Token> -> Wad<Token> -> Wad<USD>.
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// `position_value` rounded down at each step for collateral-side gate
/// valuation. Rounding slack cannot loosen LTV.
pub fn position_value_floor(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_floor(env, index);
    let actual_wad = actual.to_wad_floor();
    actual_wad.mul_floor(env, price)
}

/// `position_value` rounded up at each step for debt-side gate valuation.
/// Rounding slack cannot understate what is owed.
pub fn position_value_ceil(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_ceil(env, index);
    let actual_wad = actual.to_wad_ceil();
    actual_wad.mul_ceil(env, price)
}

/// Collateral value weighted by liquidation threshold in BPS, rounded down:
/// the health-factor numerator cannot gain from weighting rounding.
pub fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad_floor(env, value)
}

/// Sums floor-valued, LTV-weighted collateral (USD WAD) across supply positions.
pub fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    cache.prefetch_market_indexes(&supply_positions.keys());

    let mut ltv = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // Floor the whole chain: borrowing capacity cannot round upward.
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

/// Portfolio risk aggregates for borrow capacity and health-factor checks.
pub(crate) struct AccountRiskTotals {
    pub total_collateral: Wad,
    pub ltv_collateral: Wad,
    pub weighted_collateral: Wad,
    pub total_debt: Wad,
    pub health_factor: Wad,
}

/// Computes the account's collateral, debt, and health-factor risk totals.
pub fn calculate_account_risk_totals(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    _calculate_account_risk_totals_impl(env, cache, spoke_id, supply_positions, borrow_positions)
}

/// Dispatches account risk totals to the shared body in non-Certora builds.
#[cfg(not(feature = "certora"))]
fn _calculate_account_risk_totals_impl(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    calculate_account_risk_totals_body(env, cache, spoke_id, supply_positions, borrow_positions)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::calculate_account_risk_totals_summary,
    pub(crate) fn _calculate_account_risk_totals_impl(
        env: &Env,
        cache: &mut Cache,
        spoke_id: u32,
        supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
        borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    ) -> AccountRiskTotals {
        calculate_account_risk_totals_body(env, cache, spoke_id, supply_positions, borrow_positions)
    }
);

/// Prefetches feeds and market indexes, then walks the portfolio to build the risk totals.
fn calculate_account_risk_totals_body(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    // Prime the RedStone and pool-sync prefetches with every position's feeds
    // and markets before the per-asset reads below.
    let mut priced_keys = supply_positions.keys();
    priced_keys.append(&borrow_positions.keys());
    let priced_assets = position_assets(env, &priced_keys);
    oracle::prefetch_redstone_feeds(cache, &priced_assets);
    cache.prefetch_market_indexes(&priced_keys);

    let mut total_collateral = Wad::ZERO;
    let mut ltv_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // Floor before solvency gates; neutral valuation is only for proportions.
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
        ltv_collateral += position.loan_to_value.apply_to_wad_floor(env, gate_value);
        weighted_coll += weighted_collateral(env, gate_value, position.liquidation_threshold);
    }

    let mut total_debt = Wad::ZERO;
    for (hub_asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // Ceil the whole chain: owed value cannot round downward.
        total_debt += position_value_ceil(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );
    }

    let health_factor = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        weighted_coll.div_floor(env, total_debt)
    };

    AccountRiskTotals {
        total_collateral,
        ltv_collateral,
        weighted_collateral: weighted_coll,
        total_debt,
        health_factor,
    }
}
