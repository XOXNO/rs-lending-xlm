//! Bulk-prefetches RedStone feeds into the transaction cache.
//! Only raw provider payloads are cached; staleness, sanity, and tolerance
//! checks still run when a price is resolved.

use soroban_sdk::{Address, Vec};

use crate::context::PriceCache as Cache;

/// Minimum distinct feeds per adapter for bulk prefetch.
/// A single-feed bulk call can price an asset the flow does not read.
#[cfg(not(feature = "certora"))]
const MIN_BULK_FEEDS: u32 = 2;

#[cfg(feature = "certora")]
pub(crate) fn prefetch_redstone_feeds(_cache: &mut Cache, _assets: &Vec<Address>) {}

#[cfg(not(feature = "certora"))]
use soroban_sdk::{Map, String};

#[cfg(not(feature = "certora"))]
use crate::providers::redstone::read_price_data_bulk;
#[cfg(not(feature = "certora"))]
use common::types::OracleSourceConfig;

/// Bulk-fetches each adapter's RedStone feeds (only when it has at least
/// `MIN_BULK_FEEDS`) into the transaction cache.
#[cfg(not(feature = "certora"))]
pub(crate) fn prefetch_redstone_feeds(cache: &mut Cache, assets: &Vec<Address>) {
    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);

    for asset in assets.iter() {
        if cache.token_prices.contains_key(asset.clone()) {
            continue;
        }
        // Skip assets with no `AssetOracle` (pending/disabled) so prefetch never panics.
        let Some(oracle_config) = cache.cached_asset_oracle_opt(&asset) else {
            continue;
        };
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
    source: &OracleSourceConfig,
) {
    let (OracleSourceConfig::RedStone(r) | OracleSourceConfig::Xoxno(r)) = source else {
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
    if feeds.contains(&r.feed_id) {
        return;
    }
    feeds.push_back(r.feed_id.clone());
    by_adapter.set(r.contract.clone(), feeds);
}

#[cfg(all(test, not(feature = "certora")))]
mod tests {
    use crate::prefetch::*;
    use common::types::{OracleSourceConfig, RedStoneSourceConfig};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn collect_redstone_feed_dedupes_by_adapter_and_feed_id() {
        let env = Env::default();
        let cache = Cache::new_view(&env);
        let adapter = Address::generate(&env);
        let feed_id = String::from_str(&env, "BTC/USD");
        let source = OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: adapter.clone(),
            feed_id: feed_id.clone(),
            decimals: 8,
            max_stale_seconds: 900,
        });
        let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);
        collect_redstone_feed(&cache, &env, &mut by_adapter, &source);
        collect_redstone_feed(&cache, &env, &mut by_adapter, &source);

        let feeds = by_adapter.get(adapter).expect("adapter feeds");
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds.get_unchecked(0), feed_id);
    }
}
