//! Session state used while resolving one or more price keys in a single
//! call to the aggregator. Holds cycle-detection state, a cache of raw
//! provider feed payloads, and per-key resolved prices, errors, and
//! statuses so that shared sub-resolutions are not repeated within the
//! same call.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

#[cfg(not(feature = "certora"))]
use common::types::{PriceSource, ProviderRef, MAX_RESOLUTION_DEPTH};

/// Per-invocation resolution state for the price aggregator.
///
/// Caches raw provider feed payloads and per-key resolved prices, errors, and
/// statuses for the duration of a single resolution pass, and tracks which
/// keys are currently being resolved so that recursive resolution can detect
/// cycles.
pub(crate) struct Session {
    env: Env,
    feed_cache: Map<(Address, String), RedStonePriceData>,
    resolving_keys: Vec<PriceKey>,
    key_prices: Map<PriceKey, PriceFeedRaw>,
    key_errors: Map<PriceKey, OracleError>,
    key_statuses: Map<PriceKey, PriceStatus>,
    now_secs: u64,
}

impl Session {
    /// Creates an empty session, capturing the current ledger timestamp for
    /// use as the resolution time.
    pub(crate) fn new(env: &Env) -> Self {
        Session {
            env: env.clone(),
            feed_cache: Map::new(env),
            resolving_keys: Vec::new(env),
            key_prices: Map::new(env),
            key_errors: Map::new(env),
            key_statuses: Map::new(env),
            now_secs: env.ledger().timestamp(),
        }
    }

    /// Returns the session's environment handle.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Returns the ledger timestamp captured when the session was created.
    pub(crate) fn now_secs(&self) -> u64 {
        self.now_secs
    }

    /// Returns the cached raw provider payload for the given adapter and
    /// feed id, if one has already been fetched or warmed in this session.
    pub(crate) fn get_feed(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.feed_cache.get((adapter.clone(), feed_id.clone()))
    }

    /// Caches a raw provider payload for the given adapter and feed id,
    /// overwriting any existing entry.
    pub(crate) fn set_feed(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.feed_cache
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Returns whether the given key is currently on the resolution stack.
    pub(crate) fn is_resolving(&self, key: &PriceKey) -> bool {
        self.resolving_keys.iter().any(|k| k == *key)
    }

    /// Pushes a key onto the resolution stack. Panics with
    /// `OracleError::OracleCycleDetected` if the key is already on the
    /// stack.
    pub(crate) fn push_key(&mut self, key: &PriceKey) {
        if self.is_resolving(key) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving_keys.push_back(key.clone());
    }

    /// Pops the most recently pushed key off the resolution stack.
    pub(crate) fn pop_key(&mut self) {
        self.resolving_keys.pop_back();
    }

    /// Returns the resolved price previously stored for the given key in
    /// this session, if any.
    pub(crate) fn cached_price(&self, key: &PriceKey) -> Option<PriceFeedRaw> {
        self.key_prices.get(key.clone())
    }

    /// Stores the resolved price for the given key in this session.
    pub(crate) fn store_price(&mut self, key: &PriceKey, feed: PriceFeedRaw) {
        self.key_prices.set(key.clone(), feed);
    }

    /// Returns the error previously stored for the given key in this
    /// session, if any.
    pub(crate) fn cached_error(&self, key: &PriceKey) -> Option<OracleError> {
        self.key_errors.get(key.clone())
    }

    /// Stores an error for the given key in this session.
    pub(crate) fn store_error(&mut self, key: &PriceKey, error: OracleError) {
        self.key_errors.set(key.clone(), error);
    }

    /// Returns the status previously stored for the given key in this
    /// session, if any.
    pub(crate) fn cached_status(&self, key: &PriceKey) -> Option<PriceStatus> {
        self.key_statuses.get(key.clone())
    }

    /// Stores the status for the given key in this session.
    pub(crate) fn store_status(&mut self, key: &PriceKey, status: PriceStatus) {
        self.key_statuses.set(key.clone(), status);
    }

    /// No-op under the `certora` feature, where bulk warming is disabled.
    #[cfg(feature = "certora")]
    pub(crate) fn warm(&mut self, _keys: &Vec<PriceKey>) {}

    /// Pre-fetches raw provider payloads for the given keys and their
    /// transitive dependencies, grouping feed ids by adapter and issuing a
    /// bulk read per adapter that has at least `MIN_BULK_FEEDS` distinct
    /// feeds. Fetched payloads are stored in the feed cache via `set_feed`.
    /// Adapters with fewer feeds are skipped, and a failed bulk read for an
    /// adapter is skipped rather than propagated.
    #[cfg(not(feature = "certora"))]
    pub(crate) fn warm(&mut self, keys: &Vec<PriceKey>) {
        use crate::providers::multi_feed::read_price_data_bulk;

        const MIN_BULK_FEEDS: u32 = 2;

        let env = self.env.clone();
        let mut by_adapter: Map<Address, Vec<String>> = Map::new(&env);
        let mut visited: Vec<PriceKey> = Vec::new(&env);

        for key in keys.iter() {
            collect_key(&env, &mut by_adapter, &mut visited, &key, 0);
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
                    self.set_feed(&adapter, &feed_id, payload);
                }
            }
        }
    }
}

/// Walks the oracle configuration for `key` and its dependent keys (through
/// scaled and LP sources), collecting the provider feeds each one touches
/// into `by_adapter` keyed by adapter address. Stops recursing once a key
/// has already been visited or `depth` exceeds `MAX_RESOLUTION_DEPTH`, and
/// returns without collecting anything if the key has no registered oracle.
#[cfg(not(feature = "certora"))]
fn collect_key(
    env: &Env,
    by_adapter: &mut Map<Address, Vec<String>>,
    visited: &mut Vec<PriceKey>,
    key: &PriceKey,
    depth: u32,
) {
    if depth > MAX_RESOLUTION_DEPTH || visited.iter().any(|k| k == *key) {
        return;
    }
    visited.push_back(key.clone());

    let Some(oracle) = crate::registry::get_oracle(env, key) else {
        return;
    };

    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => collect_provider(env, by_adapter, &feed.provider),
            PriceSource::Scaled(scaled) => {
                collect_provider(env, by_adapter, &scaled.factor.provider);
                collect_key(env, by_adapter, visited, &scaled.quote, depth + 1);
            }

            PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => {
                collect_key(env, by_adapter, visited, &lp.key_a, depth + 1);
                collect_key(env, by_adapter, visited, &lp.key_b, depth + 1);
            }
        }
    }
}

/// Records the feed id referenced by `provider` under its adapter contract
/// in `by_adapter`, deduplicating against feeds already recorded for that
/// adapter. Reflector providers are not bulk-fetchable and are skipped.
#[cfg(not(feature = "certora"))]
fn collect_provider(env: &Env, by_adapter: &mut Map<Address, Vec<String>>, provider: &ProviderRef) {
    let multi_feed = match provider {
        ProviderRef::RedStone(feed) | ProviderRef::Xoxno(feed) => feed,
        ProviderRef::Reflector(_) => return,
    };
    let mut feeds = by_adapter
        .get(multi_feed.contract.clone())
        .unwrap_or_else(|| Vec::new(env));
    if !feeds.iter().any(|f| f == multi_feed.feed_id) {
        feeds.push_back(multi_feed.feed_id.clone());
    }
    by_adapter.set(multi_feed.contract.clone(), feeds);
}

#[cfg(test)]
#[path = "../tests/oracle/context.rs"]
mod tests;
