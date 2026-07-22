//! Health-factor, LTV, and debt aggregation.

use common::math::fp::{Bps, Ray, Wad};
use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use crate::context::Cache;
use crate::payments;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// Merge supply + borrow hub keys for bulk market-index and price prefetch.
/// Takes ownership of `supply_keys` so callers can pass `map.keys()` without an extra clone.
pub(crate) fn portfolio_hub_keys(
    mut supply_keys: Vec<HubAssetKey>,
    borrow_keys: &Vec<HubAssetKey>,
) -> Vec<HubAssetKey> {
    supply_keys.append(borrow_keys);
    supply_keys
}

/// Token addresses for bulk price-aggregator prefetch of an account's positions
/// plus optional strategy legs (order-preserving, token-unique).
pub(crate) fn account_price_assets(
    env: &Env,
    account: &Account,
    extras: &Vec<Address>,
) -> Vec<Address> {
    let mut assets = Vec::new(env);
    for key in account.supply_positions.keys().iter() {
        payments::push_unique_address(&mut assets, key.asset);
    }
    for key in account.borrow_positions.keys().iter() {
        payments::push_unique_address(&mut assets, key.asset);
    }
    for asset in extras.iter() {
        payments::push_unique_address(&mut assets, asset.clone());
    }
    assets
}

/// Neutral USD WAD value of a scaled position.
pub(crate) fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    // dimensional: Ray<Share> * Ray<Index> -> Ray<Token> -> Wad<Token> -> Wad<USD>.
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// `position_value` rounded down at each step for collateral-side gate
/// valuation. Rounding slack cannot loosen LTV.
pub(crate) fn position_value_floor(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_floor(env, index);
    let actual_wad = actual.to_wad_floor();
    actual_wad.mul_floor(env, price)
}

/// `position_value` rounded up at each step for debt-side gate valuation.
/// Rounding slack cannot understate what is owed.
pub(crate) fn position_value_ceil(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul_ceil(env, index);
    let actual_wad = actual.to_wad_ceil();
    actual_wad.mul_ceil(env, price)
}

/// Collateral value weighted by liquidation threshold in BPS, rounded down:
/// the health-factor numerator cannot gain from weighting rounding.
pub(crate) fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad_floor(env, value)
}

/// Rounding mode for USD position valuation.
///
/// Views use [`Neutral`](Self::Neutral). Solvency gates use
/// [`Floor`](Self::Floor) on collateral and [`Ceil`](Self::Ceil) on debt.
#[derive(Clone, Copy)]
pub(crate) enum PositionValueMode {
    Neutral,
    Floor,
    Ceil,
}

fn position_value_with_mode(
    env: &Env,
    mode: PositionValueMode,
    scaled: Ray,
    index: Ray,
    price: Wad,
) -> Wad {
    match mode {
        PositionValueMode::Neutral => position_value(env, scaled, index, price),
        PositionValueMode::Floor => position_value_floor(env, scaled, index, price),
        PositionValueMode::Ceil => position_value_ceil(env, scaled, index, price),
    }
}

/// Sums supply positions to USD WAD using `supply_index` and `mode`.
///
/// Loads markets for the supply keys, then walks. Prefer
/// [`sum_supply_usd_loaded`] when the caller already called
/// [`Cache::load_markets`].
pub(crate) fn sum_supply_usd(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    mode: PositionValueMode,
) -> Wad {
    cache.load_markets(&supply_positions.keys());
    sum_supply_usd_loaded(env, cache, supply_positions, mode)
}

/// Like [`sum_supply_usd`], but does not fetch — prices/indexes must already
/// be loaded for every supply key.
pub(crate) fn sum_supply_usd_loaded(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    mode: PositionValueMode,
) -> Wad {
    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total.checked_add_assign(
            env,
            position_value_with_mode(
                env,
                mode,
                position.scaled_amount,
                market_index.supply_index,
                feed.price,
            ),
        );
    }
    total
}

/// Sums debt positions to USD WAD using `borrow_index` and `mode`.
///
/// Loads markets for the debt keys, then walks. Prefer
/// [`sum_debt_usd_loaded`] when the caller already called
/// [`Cache::load_markets`].
pub(crate) fn sum_debt_usd(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    mode: PositionValueMode,
) -> Wad {
    cache.load_markets(&borrow_positions.keys());
    sum_debt_usd_loaded(env, cache, borrow_positions, mode)
}

/// Like [`sum_debt_usd`], but does not fetch — prices/indexes must already
/// be loaded for every debt key.
pub(crate) fn sum_debt_usd_loaded(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    mode: PositionValueMode,
) -> Wad {
    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total.checked_add_assign(
            env,
            position_value_with_mode(
                env,
                mode,
                position.scaled_amount,
                market_index.borrow_index,
                feed.price,
            ),
        );
    }
    total
}

/// Sums floor-valued, LTV-weighted collateral (USD WAD) across supply positions.
pub(crate) fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut Cache,
    _spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    cache.load_markets(&supply_positions.keys());

    let mut ltv = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // Floor the whole chain: borrowing capacity cannot round upward.
        let value = position_value_with_mode(
            env,
            PositionValueMode::Floor,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        ltv.checked_add_assign(env, position.loan_to_value.apply_to_wad_floor(env, value));
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

pub(crate) fn calculate_account_risk_totals(
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

/// Loads prices and market indexes, then walks the portfolio to build risk totals.
fn calculate_account_risk_totals_body(
    env: &Env,
    cache: &mut Cache,
    _spoke_id: u32,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    cache.load_markets(&portfolio_hub_keys(
        supply_positions.keys(),
        &borrow_positions.keys(),
    ));

    let mut total_collateral = Wad::ZERO;
    let mut ltv_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
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

        total_collateral.checked_add_assign(env, value);
        ltv_collateral.checked_add_assign(
            env,
            position.loan_to_value.apply_to_wad_floor(env, gate_value),
        );
        weighted_coll.checked_add_assign(
            env,
            weighted_collateral(env, gate_value, position.liquidation_threshold),
        );
    }

    // Ceil the whole chain: owed value cannot round downward.
    // Markets already loaded above — do not re-walk keys through load_markets.
    let total_debt = sum_debt_usd_loaded(env, cache, borrow_positions, PositionValueMode::Ceil);

    let health_factor = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        // A tiny debt against large collateral yields a finite but
        // unrepresentable ratio; saturate rather than revert a healthy account.
        weighted_coll.div_floor_saturating(env, total_debt)
    };

    AccountRiskTotals {
        total_collateral,
        ltv_collateral,
        weighted_collateral: weighted_coll,
        total_debt,
        health_factor,
    }
}
