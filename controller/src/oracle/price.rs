//! Cache-aware price reads. The two heavy entry points
//! ([`token_price`], [`update_asset_index`]) sit in their own file so
//! the Certora harness can swap this module wholesale without touching
//! the rest of `oracle::*`.
//!
//! Production behavior:
//!   * [`token_price`] composes the per-asset feed through the
//!     primary/anchor pipeline, applies the sanity-bound circuit
//!     breaker, then caches and returns the result.
//!   * [`update_asset_index`] simulates pool-side interest accrual on
//!     the cached sync data and returns the fresh `MarketIndex`.

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
    // Absolute price floor/ceiling — the protocol's last line of
    // defense against catastrophic oracle outputs. Required under the
    // liquidation policy, which resolves anchor deviation to the live
    // aggregator: without bounds a brief spot manipulation inside the
    // deviation band could drive liquidations. `validate_sanity_bounds`
    // enforces `0 < min < max` at config time, so the only way to reach
    // a state with `max == 0` here is direct storage tampering. Reject
    // that too — there is no legitimate disabled-bounds state in
    // production.
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
