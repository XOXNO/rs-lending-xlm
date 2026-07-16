//! USD-aggregate views.

use crate::risk;
use crate::storage;
use common::math::fp::Wad;
use soroban_sdk::Env;

use crate::context::Cache;
use crate::oracle;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// Returns the account's total supplied collateral value in USD WAD; `0` for a
/// missing account or one with no supply.
pub(crate) fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let spoke_id = match storage::try_get_account_meta(env, account_id) {
        Some(meta) => meta.spoke_id,
        None => return 0,
    };
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    // Bulk-prefetch all RedStone feeds before the per-market price reads below.
    let supply_keys = supply.keys();
    let priced_assets = risk::position_assets(env, &supply_keys);
    oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);
    cache.prefetch_market_indexes(&supply_keys);

    let mut total_collateral = Wad::ZERO;

    for (hub_asset, position) in iter_typed_positions(&supply) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // dimensional: Ray<Share> * Ray<Index> * Wad<USD/asset> -> Wad<USD>.
        let value = risk::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );
        total_collateral += value;
    }

    total_collateral.raw()
}

/// Returns the account's total debt value in USD WAD; `0` for a missing account
/// or one with no debt.
pub(crate) fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    let spoke_id = match storage::try_get_account_meta(env, account_id) {
        Some(meta) => meta.spoke_id,
        None => return 0,
    };
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    // Bulk-prefetch all RedStone feeds before the per-market price reads below.
    let borrow_keys = borrow.keys();
    let priced_assets = risk::position_assets(env, &borrow_keys);
    oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);
    cache.prefetch_market_indexes(&borrow_keys);

    let mut total_borrow = Wad::ZERO;

    for (hub_asset, position) in iter_debt_positions(&borrow) {
        let feed = cache.cached_price_for(spoke_id, &hub_asset);
        let market_index = cache.cached_market_index(&hub_asset);

        // dimensional: Ray<DebtShare> * Ray<BorrowIndex> * Wad<USD/asset> -> Wad<USD>.
        let value = risk::position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );
        total_borrow += value;
    }

    total_borrow.raw()
}

/// Returns the account's LTV-weighted collateral value in USD WAD; `0` for a
/// missing account.
pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    // Bulk-prefetch all RedStone feeds before the per-market price reads inside
    // calculate_ltv_collateral_wad.
    let priced_assets = risk::position_assets(env, &account.supply_positions.keys());
    oracle::prefetch_redstone_feeds(&mut cache, &priced_assets);
    risk::calculate_ltv_collateral_wad(env, &mut cache, account.spoke_id, &account.supply_positions)
        .raw()
}
