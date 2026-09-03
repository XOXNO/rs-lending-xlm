use cvlr::cvlr_assume;
use cvlr::nondet::nondet;

use crate::spec::ghost_prices;
use crate::types::{PriceFeedRaw, PriceStatus};
use soroban_sdk::{Address, Env, Map, Vec};

/// Both views read the memoised draw, so the price a rule reads before a call,
/// the price the call values positions with, and the price the status view
/// reports are one snapshot per asset (INV-ORACLE-03).
pub(crate) fn fetch_prices(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceFeedRaw> {
    let mut prices = Map::new(env);
    for asset in assets.iter() {
        prices.set(asset.clone(), ghost_prices::price(env, &asset));
    }
    prices
}

pub(crate) fn fetch_prices_status(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceStatus> {
    let mut statuses = Map::new(env);
    for asset in assets.iter() {
        let feed = ghost_prices::price(env, &asset);
        let stale: bool = nondet();
        let deviation: bool = nondet();
        let valid: bool = nondet();

        cvlr_assume!(!valid || (!stale && !deviation));
        statuses.set(
            asset.clone(),
            PriceStatus {
                final_wad: feed.price_wad,
                primary_wad: feed.price_wad,
                secondary_wad: feed.price_wad,
                price_timestamp: feed.timestamp,
                stale,
                deviation,
                valid,
                error_code: None,
            },
        );
    }
    statuses
}
