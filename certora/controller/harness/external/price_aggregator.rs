//! Certora harness for the controller's price-aggregator client.
//!
//! Hard `fetch_prices` always returns a positive feed — solvency / health /
//! liquidation rules that consume `cache.cached_price` are therefore
//! **oracle-success-conditional**. Soft `fetch_prices_status` uses nondet
//! stale/deviation/valid flags (no longer forced healthy).

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;

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
        let stale: bool = nondet();
        let deviation: bool = nondet();
        let valid: bool = nondet();
        // The aggregator derives `valid` from `Outcome::failure`, which returns
        // PriceFeedStale / UnsafePriceNotAllowed before it can reach success:
        // `valid` implies neither flag is set. Three independent draws would
        // otherwise admit a valid-and-stale status no aggregator can emit.
        // The converse does NOT hold — a non-positive price or a sanity-band
        // violation clears `valid` with both flags low — so `valid` stays free.
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
            },
        );
    }
    statuses
}
