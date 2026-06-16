//! Bulk prefetch of RedStone feeds into the transaction cache.
//!
//! One `read_price_data` call per adapter replaces N single-feed calls
//! (~1.27MB metered memory each). Only raw provider payloads are cached,
//! so each policy, staleness, and sanity check still runs per flow.
//! Any bulk failure leaves the cache empty and the per-feed lazy path
//! takes over unchanged. The adapter returns results index-aligned with the
//! request and fails the call on a missing feed; `get_unchecked` depends on that.

use soroban_sdk::{Address, Vec};

use crate::cache::Cache;

/// Minimum distinct feeds per adapter for bulk prefetch.
/// A single-feed bulk call can price an asset the flow does not read.
const MIN_BULK_FEEDS: u32 = 2;

/// Certora stub: lazy per-feed reads preserve semantics.
#[cfg(feature = "certora")]
pub(crate) fn prefetch_redstone_feeds(_cache: &mut Cache, _assets: &Vec<Address>) {}

#[cfg(not(feature = "certora"))]
use soroban_sdk::{Map, String};

#[cfg(not(feature = "certora"))]
use super::providers::redstone::read_price_data_bulk;

#[cfg(not(feature = "certora"))]
pub(crate) fn prefetch_redstone_feeds(cache: &mut Cache, assets: &Vec<Address>) {
    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);

    for asset in assets.iter() {
        // Feed resolved this tx: nothing left to fetch for it.
        if cache.prices_cache.contains_key(asset.clone()) {
            continue;
        }
        // Flows reject unlisted assets; prefetch must not add a panic site.
        let Some(market) = cache.try_cached_market_config(&asset) else {
            continue;
        };
        let oracle_config = market.oracle_config;
        collect_redstone_feed(cache, &env, &mut by_adapter, &oracle_config.primary);
        if let Some(anchor) = oracle_config.anchor.as_ref() {
            collect_redstone_feed(cache, &env, &mut by_adapter, anchor);
        }
    }

    for (adapter, feeds) in by_adapter.iter() {
        if feeds.len() < MIN_BULK_FEEDS {
            continue;
        }
        let Some(data) = read_price_data_bulk(&env, &adapter, &feeds) else {
            continue;
        };
        // Lengths are equal: read_price_data_bulk returns Some only when
        // data.len() == feeds.len().
        for (i, feed_id) in feeds.iter().enumerate() {
            cache.set_redstone_prefetch(&adapter, &feed_id, data.get_unchecked(i as u32));
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_redstone_feed(
    cache: &Cache,
    env: &soroban_sdk::Env,
    by_adapter: &mut Map<Address, Vec<String>>,
    source: &controller_interface::types::OracleSourceConfig,
) {
    let controller_interface::types::OracleSourceConfig::RedStone(r) = source else {
        return;
    };
    if cache
        .get_redstone_prefetch(&r.contract, &r.feed_id)
        .is_some()
    {
        return;
    }
    let mut feeds = by_adapter
        .get(r.contract.clone())
        .unwrap_or_else(|| Vec::new(env));
    if feeds.first_index_of(r.feed_id.clone()).is_some() {
        return;
    }
    feeds.push_back(r.feed_id.clone());
    by_adapter.set(r.contract.clone(), feeds);
}
