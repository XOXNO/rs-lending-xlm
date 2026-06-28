//! Public price entry point (`token_price`).

use common::errors::{GenericError, OracleError};
use controller_interface::types::{MarketStatus, OracleSourceConfig, PriceFeedRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Address};

use super::compose;
use crate::cache::Cache;

pub fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    if let Some(feed) = cache.prices_cache.get(asset.clone()) {
        return feed;
    }

    let market = cache.cached_market_config(asset);
    match market.status {
        MarketStatus::PendingOracle | MarketStatus::Disabled => {
            panic_with_error!(cache.env(), GenericError::PairNotActive);
        }
        _ => {}
    }

    let config = market.oracle_config;

    // Reject the `MarketOracleConfig::pending_for` self-pointer sentinel.
    let primary_contract = match &config.primary {
        OracleSourceConfig::Reflector(r) => &r.contract,
        OracleSourceConfig::RedStone(r) => &r.contract,
    };
    assert_with_error!(
        cache.env(),
        primary_contract != asset,
        OracleError::OracleNotConfigured
    );
    let resolved = compose::resolve_components(cache, &config);
    assert_with_error!(
        cache.env(),
        resolved.final_price_wad > 0,
        OracleError::InvalidPrice
    );
    if config.max_sanity_price_wad <= 0
        || resolved.final_price_wad < config.min_sanity_price_wad
        || resolved.final_price_wad > config.max_sanity_price_wad
    {
        panic_with_error!(cache.env(), OracleError::SanityBoundViolated);
    }
    let feed = PriceFeedRaw {
        price_wad: resolved.final_price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    };

    cache.prices_cache.set(asset.clone(), feed.clone());
    feed
}
