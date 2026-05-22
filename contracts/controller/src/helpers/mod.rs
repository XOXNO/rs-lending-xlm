use common::constants::WAD;
use common::math::fp::{Bps, Ray, Wad};
use common::types::AccountPositionRaw;
use soroban_sdk::{Address, Env, Map};

use crate::cache::ControllerCache;
use crate::storage::iter_typed_positions;

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


