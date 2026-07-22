//! Bulk-warms multi-feed adapter payloads (RedStone/Xoxno wire ABI) into the
//! transaction cache. Only raw provider payloads are cached; staleness, sanity,
//! and tolerance checks still run when a price is resolved.

use soroban_sdk::{Address, Vec};

use crate::context::ResolutionContext;

#[cfg(not(feature = "certora"))]
use common::types::OracleSourceConfig;
#[cfg(not(feature = "certora"))]
use soroban_sdk::{Map, String};

#[cfg(feature = "certora")]
pub(crate) fn warm_multi_feed_adapters(_cache: &mut ResolutionContext, _assets: &Vec<Address>) {}

/// Bulk-fetches each multi-feed adapter's feeds into the transaction cache.
/// Adapters with a single feed are skipped: a bulk call of one feed costs the
/// same cross-call as the lazy per-feed read, which only fires if the price is
/// actually resolved.
#[cfg(not(feature = "certora"))]
pub(crate) fn warm_multi_feed_adapters(cache: &mut ResolutionContext, assets: &Vec<Address>) {
    use crate::providers::multi_feed::read_price_data_bulk;

    /// Minimum distinct feeds per adapter for bulk prefetch.
    const MIN_BULK_FEEDS: u32 = 2;

    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);

    for asset in assets.iter() {
        if cache.has_price(&asset) {
            continue;
        }
        // Skip assets with no `AssetOracle` (pending/disabled) so prefetch never panics.
        let Some(oracle_config) = cache.cached_asset_oracle_opt(&asset) else {
            continue;
        };
        collect_multi_feed(cache, &mut by_adapter, &oracle_config.primary);
        if let Some(anchor) = oracle_config.anchor.as_ref() {
            collect_multi_feed(cache, &mut by_adapter, anchor);
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
            cache.set_bulk_feed(&adapter, &feed_id, data.get_unchecked(i as u32));
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_multi_feed(
    cache: &ResolutionContext,
    by_adapter: &mut Map<Address, Vec<String>>,
    source: &OracleSourceConfig,
) {
    let (OracleSourceConfig::RedStone(r) | OracleSourceConfig::Xoxno(r)) = source else {
        return;
    };
    if cache.get_bulk_feed(&r.contract, &r.feed_id).is_some() {
        return;
    }
    let mut feeds = by_adapter
        .get(r.contract.clone())
        .unwrap_or_else(|| Vec::new(cache.env()));
    if feeds.contains(&r.feed_id) {
        return;
    }
    feeds.push_back(r.feed_id.clone());
    by_adapter.set(r.contract.clone(), feeds);
}

#[cfg(all(test, not(feature = "certora")))]
mod tests {
    use crate::prefetch::*;
    use common::types::RedStoneSourceConfig;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn collect_multi_feed_dedupes_by_adapter_and_feed_id() {
        let env = Env::default();
        let cache = ResolutionContext::new(&env);
        let adapter = Address::generate(&env);
        let feed_id = String::from_str(&env, "BTC/USD");
        let source = OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: adapter.clone(),
            feed_id: feed_id.clone(),
            decimals: 8,
            max_stale_seconds: 900,
        });
        let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);
        collect_multi_feed(&cache, &mut by_adapter, &source);
        collect_multi_feed(&cache, &mut by_adapter, &source);

        let feeds = by_adapter.get(adapter).expect("adapter feeds");
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds.get_unchecked(0), feed_id);
    }
}
