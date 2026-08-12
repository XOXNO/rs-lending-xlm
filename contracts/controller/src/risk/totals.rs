use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Address, Env, Map, Vec};

use common::collections::push_unique_address;

use crate::context::Cache;
use crate::storage::{iter_debt_positions, iter_typed_positions};

pub(crate) use common::rates::{position_value, position_value_ceil, position_value_floor};

pub(crate) fn portfolio_hub_keys(
    mut supply_keys: Vec<HubAssetKey>,
    borrow_keys: &Vec<HubAssetKey>,
) -> Vec<HubAssetKey> {
    supply_keys.append(borrow_keys);
    supply_keys
}

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

pub(crate) fn sum_debt_usd(
    env: &Env,
    cache: &mut Cache,
    borrow_positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    cache.load_markets(&borrow_positions.keys());
    sum_debt_usd_loaded(env, cache, borrow_positions, position_value)
}

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
        weighted_coll = weighted_coll.checked_add(
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
