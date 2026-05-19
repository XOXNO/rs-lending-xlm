//! USD-aggregate views.

use common::fp::{Ray, Wad};
use soroban_sdk::Env;

use crate::cache::ControllerCache;
use crate::{helpers, storage};

pub fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = ControllerCache::new_view(env);
    let mut total_collateral = Wad::ZERO;

    for (asset, position) in supply.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total_collateral += value;
    }

    total_collateral.raw()
}

pub fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_borrow_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = ControllerCache::new_view(env);
    let mut total_borrow = Wad::ZERO;

    for (asset, position) in borrow.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total_borrow += value;
    }

    total_borrow.raw()
}

pub fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = ControllerCache::new_view(env);
    helpers::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
}
