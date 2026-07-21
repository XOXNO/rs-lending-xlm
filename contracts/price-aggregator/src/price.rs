//! Public price entry point (`token_price`).

use common::errors::OracleError;
use common::types::{MarketOracleConfig, OracleSourceConfig, PriceFeedRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Address};

use crate::context::PriceCache as Cache;
use crate::compose;

/// Cached USD price; miss resolves under cycle guard.
pub(crate) fn token_price(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    if let Some(feed) = cache.token_prices.get(asset.clone()) {
        return feed;
    }

    // Cycle guard: re-entry of an in-flight asset reverts `OracleCycleDetected`.
    cache.enter_price_resolution(asset);

    // Missing `AssetOracle` → `OracleNotConfigured` (pending/disabled gate).
    let config = cache.cached_asset_oracle(asset);
    let feed = price_with_config(cache, asset, &config);
    cache.token_prices.set(asset.clone(), feed.clone());
    cache.exit_price_resolution();
    feed
}

/// Resolves a USD price without writing a cache entry.
pub(crate) fn price_with_config(
    cache: &mut Cache,
    asset: &Address,
    config: &MarketOracleConfig,
) -> PriceFeedRaw {
    // Reject the `MarketOracleConfig::pending_for` self-pointer sentinel.
    let primary_contract = match &config.primary {
        OracleSourceConfig::Reflector(r) => &r.contract,
        OracleSourceConfig::RedStone(r) | OracleSourceConfig::Xoxno(r) => &r.contract,
    };
    assert_with_error!(
        cache.env(),
        primary_contract != asset,
        OracleError::OracleNotConfigured
    );
    let resolved = compose::resolve_components(cache, config);
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
    PriceFeedRaw {
        price_wad: resolved.final_price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    }
}
