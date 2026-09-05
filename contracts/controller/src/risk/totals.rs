use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use common::collections::push_unique_address;

use crate::context::Context;
use crate::storage::{iter_debt_positions, iter_typed_positions};

pub(crate) use common::rates::{position_value, position_value_ceil, position_value_floor};

/// Combines supply and debt hub-asset keys without deduplication.
pub(crate) fn portfolio_hub_keys(
    mut supply_keys: Vec<HubAssetKey>,
    borrow_keys: &Vec<HubAssetKey>,
) -> Vec<HubAssetKey> {
    supply_keys.append(borrow_keys);
    supply_keys
}

/// Returns distinct token addresses from both position maps and `extras`.
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

/// Values debt in USD WAD with `value`; prices must already be cached.
fn sum_debt_usd_loaded(
    env: &Env,
    cache: &mut Context,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    value: fn(&Env, Ray, Ray, Wad) -> Wad,
) -> Wad {
    let mut total = Wad::ZERO;
    for (hub_asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        total = total.checked_add(
            env,
            value(
                env,
                position.scaled_amount,
                market_index.borrow_index,
                feed.price,
            ),
        );
    }
    total
}

/// Loads missing market data and sums debt in USD WAD with half-up rounding.
pub(crate) fn sum_debt_usd(
    env: &Env,
    cache: &mut Context,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    cache.load_markets(&borrow_positions.keys());
    sum_debt_usd_loaded(env, cache, borrow_positions, position_value)
}

/// Sums collateral in USD WAD, flooring valuation and risk weighting.
/// Uses each position's stored `min(LTV, liquidation_threshold)`.
pub(crate) fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut Context,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    cache.load_markets(&supply_positions.keys());

    let mut ltv = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let value = position_value_floor(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        let effective_ltv = position.loan_to_value.min(position.liquidation_threshold);
        ltv = ltv.checked_add(env, effective_ltv.apply_to_wad_floor(env, value));
    }
    ltv
}

/// USD totals and unitless health factor, all WAD-scaled.
pub(crate) struct AccountRiskTotals {
    /// Unweighted collateral, rounded half-up.
    pub total_collateral: Wad,
    /// Floored collateral weighted by stored `min(LTV, liquidation_threshold)`.
    pub ltv_collateral: Wad,
    /// Floored collateral weighted by stored liquidation thresholds.
    pub weighted_collateral: Wad,
    /// Debt valued with ceiling rounding.
    pub total_debt: Wad,
    /// Weighted collateral / debt; saturated maximum when debt-free.
    pub health_factor: Wad,
}

/// Computes WAD risk totals using stored position parameters. Production,
/// health rules, and solvency rules use the real arithmetic so valuations
/// match the values used by the risk gates.
#[cfg(any(
    not(feature = "certora"),
    feature = "certora-health-rules",
    feature = "certora-solvency-rules"
))]
pub(crate) fn calculate_account_risk_totals(
    env: &Env,
    cache: &mut Context,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    calculate_account_risk_totals_body(env, cache, supply_positions, borrow_positions)
}

// Other Certora rules use a nondeterministic summary to bound proof complexity.
// The macro also exposes the real body under the same-named module for index rules.
#[cfg(all(
    feature = "certora",
    not(feature = "certora-health-rules"),
    not(feature = "certora-solvency-rules")
))]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::calculate_account_risk_totals_summary,
    /// Computes WAD risk totals; substituted by a summary in this Certora build.
    pub(crate) fn calculate_account_risk_totals(
        env: &Env,
        cache: &mut Context,
        supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
        borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    ) -> AccountRiskTotals {
        calculate_account_risk_totals_body(env, cache, supply_positions, borrow_positions)
    }
);

/// Loads missing market data and values stored position risk snapshots in WAD.
/// Total collateral rounds half-up; collateral used for risk gates and its
/// weights round down, while debt rounds up. Health factor is weighted
/// collateral / debt, floored and saturated at `i128::MAX`; debt-free accounts
/// use `i128::MAX`.
fn calculate_account_risk_totals_body(
    env: &Env,
    cache: &mut Context,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> AccountRiskTotals {
    cache.load_markets(&portfolio_hub_keys(
        supply_positions.keys(),
        &borrow_positions.keys(),
    ));

    let mut total_collateral = Wad::ZERO;
    let mut ltv_collateral = Wad::ZERO;
    let mut weighted_collateral = Wad::ZERO;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

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
        // A gated threshold can stay below refreshed LTV; clamp the borrow limit to it.
        let effective_ltv = position.loan_to_value.min(position.liquidation_threshold);
        ltv_collateral =
            ltv_collateral.checked_add(env, effective_ltv.apply_to_wad_floor(env, gate_value));
        weighted_collateral = weighted_collateral.checked_add(
            env,
            position
                .liquidation_threshold
                .apply_to_wad_floor(env, gate_value),
        );
    }

    let total_debt = sum_debt_usd_loaded(env, cache, borrow_positions, position_value_ceil);

    let health_factor = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        weighted_collateral.div_floor_saturating(env, total_debt)
    };

    AccountRiskTotals {
        total_collateral,
        ltv_collateral,
        weighted_collateral,
        total_debt,
        health_factor,
    }
}
