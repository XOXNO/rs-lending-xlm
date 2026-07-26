//! Health-factor, LTV, and debt aggregation.

use common::math::fp::{Bps, Ray, Wad};
use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use common::collections::push_unique_address;

use crate::context::Cache;
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
        push_unique_address(&mut assets, key.asset);
    }
    for key in account.borrow_positions.keys().iter() {
        push_unique_address(&mut assets, key.asset);
    }
    for asset in extras.iter() {
        push_unique_address(&mut assets, asset.clone());
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

/// Neutrally-valued supply total in USD WAD, fetching prices and indexes first.
///
/// Neutral rounding is for reporting only; solvency gates use the floor/ceil
/// valuations in [`calculate_account_risk_totals`].
pub(crate) fn sum_supply_usd(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    cache.load_markets(&supply_positions.keys());

    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total = total.checked_add(
            env,
            position_value(
                env,
                position.scaled_amount,
                market_index.supply_index,
                feed.price,
            ),
        );
    }
    total
}

/// Neutrally-valued debt total in USD WAD, fetching prices and indexes first.
pub(crate) fn sum_debt_usd(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    cache.load_markets(&borrow_positions.keys());

    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total = total.checked_add(
            env,
            position_value(
                env,
                position.scaled_amount,
                market_index.borrow_index,
                feed.price,
            ),
        );
    }
    total
}

/// Ceil-valued debt total for solvency gates: owed value cannot round downward.
/// Does not fetch — prices and indexes must already be loaded for every key.
fn sum_debt_usd_ceil_loaded(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total = total.checked_add(
            env,
            position_value_ceil(
                env,
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
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    cache.load_markets(&supply_positions.keys());

    let mut ltv = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // Floor the whole chain: borrowing capacity cannot round upward.
        let value = position_value_floor(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        ltv = ltv.checked_add(env, position.loan_to_value.apply_to_wad_floor(env, value));
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

/// Loads prices and market indexes, then walks the portfolio to build risk totals.
///
/// The portfolio walk is unbounded for the prover, so the `certora` build
/// swaps this entry point for a nondeterministic summary; both builds keep
/// `calculate_account_risk_totals_body` as the single real aggregation.
#[cfg(not(feature = "certora"))]
pub(crate) fn calculate_account_risk_totals(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    calculate_account_risk_totals_body(env, cache, supply_positions, borrow_positions)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::calculate_account_risk_totals_summary,
    pub(crate) fn calculate_account_risk_totals(
        env: &Env,
        cache: &mut Cache,
        supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
        borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    ) -> AccountRiskTotals {
        calculate_account_risk_totals_body(env, cache, supply_positions, borrow_positions)
    }
);

/// Prices every supply leg into neutral, LTV-weighted, and threshold-weighted
/// collateral, ceils total debt, and derives the health factor.
fn calculate_account_risk_totals_body(
    env: &Env,
    cache: &mut Cache,
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

        total_collateral = total_collateral.checked_add(env, value);
        ltv_collateral = ltv_collateral.checked_add(
            env,
            position.loan_to_value.apply_to_wad_floor(env, gate_value),
        );
        weighted_coll = weighted_coll.checked_add(
            env,
            weighted_collateral(env, gate_value, position.liquidation_threshold),
        );
    }

    // Ceil the whole chain: owed value cannot round downward.
    // Markets already loaded above — do not re-walk keys through load_markets.
    let total_debt = sum_debt_usd_ceil_loaded(env, cache, borrow_positions);

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
