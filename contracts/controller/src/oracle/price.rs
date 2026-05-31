//! Public price & index entry points (`token_price`, `update_asset_index`).
//!
//! These are the two functions re-exported from the oracle module and
//! called by the cache. They are the integration point between the
//! full oracle resolution pipeline and the rest of the lending logic.

use common::errors::{GenericError, OracleError};
use common::rates::simulate_update_indexes;
use common::types::{MarketIndex, MarketStatus, OracleSourceConfig, PriceFeedRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Address};

use super::compose;
use crate::cache::Cache;

pub fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
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
    let resolved = compose::resolve_price(cache, &config);
    assert_with_error!(
        cache.env(),
        resolved.price_wad > 0,
        OracleError::InvalidPrice
    );
    // Sanity price bounds.
    if config.max_sanity_price_wad <= 0
        || resolved.price_wad < config.min_sanity_price_wad
        || resolved.price_wad > config.max_sanity_price_wad
    {
        panic_with_error!(cache.env(), OracleError::SanityBoundViolated);
    }
    let feed = PriceFeedRaw {
        price_wad: resolved.price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    };

    cache.prices_cache.set(asset.clone(), feed.clone());
    feed
}

pub fn update_asset_index(cache: &mut Cache, asset: &Address) -> MarketIndex {
    let sync_data = cache.cached_pool_sync_data(asset);
    simulate_update_indexes(cache.env(), cache.current_timestamp_ms, &sync_data)
}
