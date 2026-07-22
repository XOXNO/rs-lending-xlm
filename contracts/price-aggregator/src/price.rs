//! Public price entry point (`resolve_usd_price`).

use common::errors::OracleError;
use common::types::{AssetOracleConfig, PriceFeedRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Address};

use crate::compose::{self, ResolvedPrice};
use crate::context::ResolutionContext;

/// Cached USD price; miss resolves under cycle guard.
pub(crate) fn resolve_usd_price(cache: &mut ResolutionContext, asset: &Address) -> PriceFeedRaw {
    if let Some(feed) = cache.cached_price(asset) {
        return feed;
    }

    // Cycle guard: re-entry of an in-flight asset reverts `OracleCycleDetected`.
    cache.push_resolution(asset);

    // Missing `AssetOracle` → `OracleNotConfigured` (pending/disabled gate).
    let config = cache.cached_asset_oracle(asset);
    let feed = resolve_with_config(cache, asset, &config);
    cache.store_price(asset, feed.clone());
    cache.pop_resolution();
    feed
}

/// Resolves a USD price without writing a cache entry.
pub(crate) fn resolve_with_config(
    cache: &mut ResolutionContext,
    asset: &Address,
    config: &AssetOracleConfig,
) -> PriceFeedRaw {
    let resolved = resolve_guarded(cache, asset, config);
    PriceFeedRaw {
        price_wad: resolved.final_price_wad,
        asset_decimals: config.asset_decimals,
        timestamp: resolved.timestamp,
    }
}

/// Resolves oracle components and applies every fail-closed check: pending
/// sentinel, positive final price, and the configured sanity band.
pub(crate) fn resolve_guarded(
    cache: &mut ResolutionContext,
    asset: &Address,
    config: &AssetOracleConfig,
) -> ResolvedPrice {
    // Reject the `AssetOracleConfig::pending_for` self-pointer sentinel.
    assert_with_error!(
        cache.env(),
        !config.is_pending(asset),
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
    resolved
}
