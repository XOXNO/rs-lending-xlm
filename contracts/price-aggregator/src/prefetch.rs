//! Bulk-warms multi-feed adapter payloads (RedStone/Xoxno wire ABI) into the
//! transaction cache. Only raw provider payloads are cached; staleness, sanity,
//! and agreement checks still run when a price is resolved.
//!
//! This is a cost optimization, not a correctness mechanism. It matters because
//! the controller prices a whole portfolio in one call: without it, an account
//! holding eight assets on one adapter pays eight cross-contract calls where one
//! would do, and the budget it saves is the budget a liquidation runs on.

use soroban_sdk::{Address, Vec};

use crate::context::ResolutionContext;
use common::types::PriceKey;

#[cfg(not(feature = "certora"))]
use common::types::{PriceSource, ProviderRef, MAX_RESOLUTION_DEPTH};
#[cfg(not(feature = "certora"))]
use soroban_sdk::{Map, String};

/// No-op under `--features certora`: the summarized provider reads never
/// consult the bulk cache, so warming it would only add unreachable state.
#[cfg(feature = "certora")]
pub(crate) fn warm_multi_feed_adapters(_cache: &mut ResolutionContext, _keys: &Vec<PriceKey>) {}

/// Bulk-fetches each multi-feed adapter's feeds into the transaction cache.
///
/// Walks composition, so a feed reached only through a `Scaled` quote or an LP
/// leg is warmed too — those are exactly the assets whose resolution costs the
/// most calls, so skipping them would leave the optimization pointed at the
/// cheap cases.
///
/// Adapters with a single feed are skipped: a bulk call of one feed costs the
/// same cross-call as the lazy per-feed read, which only fires if the price is
/// actually resolved.
#[cfg(not(feature = "certora"))]
pub(crate) fn warm_multi_feed_adapters(cache: &mut ResolutionContext, keys: &Vec<PriceKey>) {
    use crate::providers::multi_feed::read_price_data_bulk;

    /// Minimum distinct feeds per adapter for bulk prefetch.
    const MIN_BULK_FEEDS: u32 = 2;

    let env = cache.env().clone();
    let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);
    let mut visited: Vec<PriceKey> = Vec::new(&env);

    for key in keys.iter() {
        collect_key(cache, &mut by_adapter, &mut visited, &key, 0);
    }

    for (adapter, feeds) in by_adapter.iter() {
        if feeds.len() < MIN_BULK_FEEDS {
            continue;
        }
        let Some(payloads) = read_price_data_bulk(&env, &adapter, &feeds) else {
            continue;
        };
        for (index, feed_id) in feeds.iter().enumerate() {
            if let Some(payload) = payloads.get(index as u32) {
                cache.set_bulk_feed(&adapter, &feed_id, payload);
            }
        }
    }
}

/// Accumulates every multi-feed `(adapter, feed_id)` reachable from `key`.
///
/// Bounded by the same depth cap the resolver uses and by a visited set, so a
/// misconfigured cycle costs a bounded walk here rather than recursing until the
/// budget runs out. Prefetch must never be the thing that reverts a price call.
#[cfg(not(feature = "certora"))]
fn collect_key(
    cache: &mut ResolutionContext,
    by_adapter: &mut Map<Address, Vec<String>>,
    visited: &mut Vec<PriceKey>,
    key: &PriceKey,
    depth: u32,
) {
    if depth > MAX_RESOLUTION_DEPTH || visited.iter().any(|k| k == *key) {
        return;
    }
    visited.push_back(key.clone());

    let env = cache.env().clone();
    // A key with no config is simply skipped: prefetch is best-effort and the
    // resolver reports the missing config with a precise error.
    let Some(oracle) = crate::registry::resolve_oracle(&env, key) else {
        return;
    };

    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => collect_provider(&env, by_adapter, &feed.provider),
            PriceSource::Scaled(scaled) => {
                collect_provider(&env, by_adapter, &scaled.factor.provider);
                collect_key(cache, by_adapter, visited, &scaled.quote, depth + 1);
            }
            PriceSource::LpShare(lp) => {
                collect_key(cache, by_adapter, visited, &lp.key_a, depth + 1);
                collect_key(cache, by_adapter, visited, &lp.key_b, depth + 1);
            }
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_provider(
    env: &soroban_sdk::Env,
    by_adapter: &mut Map<Address, Vec<String>>,
    provider: &ProviderRef,
) {
    let ProviderRef::MultiFeed(multi_feed) = provider else {
        return;
    };
    let mut feeds = by_adapter
        .get(multi_feed.contract.clone())
        .unwrap_or_else(|| Vec::new(env));
    if !feeds.iter().any(|f| f == multi_feed.feed_id) {
        feeds.push_back(multi_feed.feed_id.clone());
    }
    by_adapter.set(multi_feed.contract.clone(), feeds);
}
