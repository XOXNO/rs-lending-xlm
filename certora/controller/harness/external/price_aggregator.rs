//! Certora harness for the controller's price-aggregator client.
//! Every requested asset receives one positive, bounded price feed.

use crate::spec::summaries::price_feed_summary;
use crate::types::{PriceFeedRaw, PriceStatus};
use soroban_sdk::{Address, Env, Map, Vec};

pub(crate) fn fetch_prices(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceFeedRaw> {
    let mut prices = Map::new(env);
    for asset in assets.iter() {
        prices.set(asset.clone(), price_feed_summary(env, &asset));
    }
    prices
}

pub(crate) fn fetch_prices_status(env: &Env, assets: &Vec<Address>) -> Map<Address, PriceStatus> {
    let mut statuses = Map::new(env);
    for asset in assets.iter() {
        let feed = price_feed_summary(env, &asset);
        statuses.set(
            asset.clone(),
            PriceStatus {
                final_wad: feed.price_wad,
                primary_wad: feed.price_wad,
                secondary_wad: feed.price_wad,
                price_timestamp: feed.timestamp,
                stale: false,
                deviation: false,
                valid: true,
            },
        );
    }
    statuses
}
