use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use common::collections::push_unique_address;

use crate::context::Cache;
use crate::storage::{iter_debt_positions, iter_typed_positions};

pub(crate) use common::rates::{position_value, position_value_ceil, position_value_floor};

/// Appends `borrow_keys` onto `supply_keys` and returns the combined list of
/// hub-asset keys.
pub(crate) fn portfolio_hub_keys(
    mut supply_keys: Vec<HubAssetKey>,
    borrow_keys: &Vec<HubAssetKey>,
) -> Vec<HubAssetKey> {
    supply_keys.append(borrow_keys);
    supply_keys
}

/// Returns the deduplicated underlying asset addresses referenced by
/// `account`'s supply positions, borrow positions, and `extras`.
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

/// Sums `borrow_positions`' USD value (WAD) using `value` as the per-position
/// valuation function, assuming market data is already cached.
fn sum_debt_usd_loaded(
    env: &Env,
    cache: &mut Cache,
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

/// Loads market data for `borrow_positions` into `cache`, then sums their
/// USD value (WAD) using half-up rounding.
pub(crate) fn sum_debt_usd(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    cache.load_markets(&borrow_positions.keys());
    sum_debt_usd_loaded(env, cache, borrow_positions, position_value)
}

/// Loads market data for `supply_positions` into `cache`, then sums each
/// position's floor-valued collateral, floor-scaled by the lesser of its
/// loan-to-value and liquidation threshold (WAD).
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

pub(crate) struct AccountRiskTotals {
    pub total_collateral: Wad,
    pub ltv_collateral: Wad,
    pub weighted_collateral: Wad,
    pub total_debt: Wad,
    pub health_factor: Wad,
}

/// Computes an account's aggregate risk totals (collateral, debt, and
/// health factor, all WAD) for `supply_positions` and `borrow_positions`.
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
    /// Computes an account's aggregate risk totals (collateral, debt, and
    /// health factor, all WAD) for `supply_positions` and `borrow_positions`.
    pub(crate) fn calculate_account_risk_totals(
        env: &Env,
        cache: &mut Cache,
        supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
        borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
    ) -> AccountRiskTotals {
        calculate_account_risk_totals_body(env, cache, supply_positions, borrow_positions)
    }
);

/// Loads market data for `supply_positions` and `borrow_positions` into
/// `cache`, then computes total collateral, LTV-gated collateral,
/// liquidation-threshold-weighted collateral, total debt, and health factor
/// (all WAD). Debt is valued with ceiling rounding and the collateral
/// feeding the LTV and weighted sums is floored. Health factor is
/// `i128::MAX` when there is no debt, otherwise weighted collateral divided
/// by total debt, floored and saturating at `i128::MAX` instead of
/// overflowing.
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
        // A gated tuple can leave a position with LTV above its frozen LT;
        // clamp so the origination buffer cannot invert.
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
