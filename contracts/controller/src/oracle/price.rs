use common::errors::{GenericError, OracleError};
use common::rates::simulate_update_indexes;
use common::types::{MarketIndex, MarketStatus, PriceFeed};
use soroban_sdk::{panic_with_error, Address};

use super::compose;
use crate::cache::ControllerCache;

pub fn token_price(cache: &mut ControllerCache, asset: &Address) -> PriceFeed {
    if let Some(feed) = cache.prices_cache.get(asset.clone()) {
        return feed;
    }

    let market = cache.cached_market_config(asset);
    match market.status {
        MarketStatus::PendingOracle => {
            panic_with_error!(cache.env(), GenericError::PairNotActive);
        }
        MarketStatus::Disabled if !cache.oracle_policy.allows_disabled_market() => {
            panic_with_error!(cache.env(), GenericError::PairNotActive);
        }
        _ => {}
    }

    let config = market.oracle_config;
    let resolved = compose::resolve_price(cache, &config);
    if resolved.price_wad <= 0 {
        panic_with_error!(cache.env(), OracleError::InvalidPrice);
    }
    // Sanity price bounds.
    if config.max_sanity_price_wad <= 0
        || resolved.price_wad < config.min_sanity_price_wad
        || resolved.price_wad > config.max_sanity_price_wad
    {
        panic_with_error!(cache.env(), OracleError::SanityBoundViolated);
    }
    let feed = PriceFeed {
        price_wad: resolved.price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    };

    cache.prices_cache.set(asset.clone(), feed.clone());
    feed
}

pub fn update_asset_index(cache: &mut ControllerCache, asset: &Address) -> MarketIndex {
    let env = cache.env().clone();
    let sync_data = cache.cached_pool_sync_data(asset);
    simulate_update_indexes(&env, cache.current_timestamp_ms, &sync_data)
}
