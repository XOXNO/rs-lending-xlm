//! USD-aggregate views.

use common::math::fp::Wad;
use soroban_sdk::Env;

use crate::cache::Cache;
use crate::storage::{iter_debt_positions, iter_typed_positions};
use crate::{helpers, storage};

pub fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    let mut total_collateral = Wad::ZERO;

    for (asset, position) in iter_typed_positions(&supply) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );
        total_collateral += value;
    }

    total_collateral.raw()
}

pub fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    let mut total_borrow = Wad::ZERO;

    for (asset, position) in iter_debt_positions(&borrow) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
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
    let mut cache = Cache::new_view(env);
    helpers::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
}
