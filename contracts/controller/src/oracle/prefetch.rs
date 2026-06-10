//! Bulk prefetch of RedStone feeds into the transaction cache.
//!
//! One `read_price_data` call per adapter replaces N single-feed calls
//! (~1.27MB metered memory each). Only raw provider payloads are cached,
//! so every policy, staleness, and sanity check still runs per flow.
//! Any bulk failure leaves the cache empty and the per-feed lazy path
//! takes over unchanged. The real adapter returns results index-aligned
//! with the request and fails whole-call on a missing feed (verified
//! on-chain); a length-checked zip relies on that.

use soroban_sdk::{Address, Vec};

use crate::cache::Cache;

/// Below this many distinct feeds per adapter, bulk saves nothing.
const MIN_BULK_FEEDS: u32 = 2;

/// No-op under Certora: pure performance optimization, identical semantics.
#[cfg(feature = "certora")]
pub(crate) fn prefetch_redstone_feeds(_cache: &mut Cache, _assets: &Vec<Address>) {}

#[cfg(not(feature = "certora"))]
pub(crate) fn prefetch_redstone_feeds(cache: &mut Cache, assets: &Vec<Address>) {
    use soroban_sdk::{Map, String};

    use super::providers::redstone::read_price_data_bulk;

    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);

    for asset in assets.iter() {
        // Already fully resolved this tx: nothing left to fetch for it.
        if cache.prices_cache.contains_key(asset.clone()) {
            continue;
        }
        let oracle_config = cache.cached_market_config(&asset).oracle_config;
        collect_redstone_feed(&*cache, &env, &mut by_adapter, &oracle_config.primary);
        if let Some(anchor) = oracle_config.anchor.as_ref() {
            collect_redstone_feed(&*cache, &env, &mut by_adapter, anchor);
        }
    }

    for (adapter, feeds) in by_adapter.iter() {
        if feeds.len() < MIN_BULK_FEEDS {
            continue;
        }
        let Some(data) = read_price_data_bulk(&env, &adapter, &feeds) else {
            continue;
        };
        // Lengths match (checked in read_price_data_bulk); zip by index.
        for (i, feed_id) in feeds.iter().enumerate() {
            if let Some(entry) = data.get(i as u32) {
                cache.set_redstone_prefetch(&adapter, &feed_id, entry);
            }
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_redstone_feed(
    cache: &Cache,
    env: &soroban_sdk::Env,
    by_adapter: &mut soroban_sdk::Map<Address, soroban_sdk::Vec<soroban_sdk::String>>,
    source: &common::types::OracleSourceConfig,
) {
    let common::types::OracleSourceConfig::RedStone(r) = source else {
        return;
    };
    if cache.get_redstone_prefetch(&r.contract, &r.feed_id).is_some() {
        return;
    }
    let mut feeds = by_adapter
        .get(r.contract.clone())
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));
    if feeds.first_index_of(r.feed_id.clone()).is_some() {
        return;
    }
    feeds.push_back(r.feed_id.clone());
    by_adapter.set(r.contract.clone(), feeds);
}
