//! Transaction-local pricing state: ledger clock, cycle stack, multi-feed
//! payload cache, and per-key price / status memos.
//!
//! Bulk `prices` / `quotes` call [`Session::warm`] once, then resolve each key.
//! Multi-feed adapters may be bulk-fetched by contract address when a call
//! needs two or more distinct feed ids; Reflector has no bulk path. The feed
//! cache also stores lazy single multi-feed reads within the same session.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

#[cfg(not(feature = "certora"))]
use common::types::{PriceSource, ProviderRef, MAX_RESOLUTION_DEPTH};

/// Transaction-local state for one pricing call.
pub(crate) struct Session {
    env: Env,
    /// Multi-feed payloads filled by [`Self::warm`] and by lazy single reads.
    feed_cache: Map<(Address, String), RedStonePriceData>,
    resolving_keys: Vec<PriceKey>,
    key_prices: Map<PriceKey, PriceFeedRaw>,
    key_errors: Map<PriceKey, OracleError>,
    key_statuses: Map<PriceKey, PriceStatus>,
    now_secs: u64,
}

impl Session {
    /// Snapshot the ledger timestamp and empty caches / stacks.
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

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Ledger timestamp captured at [`Self::new`].
    pub(crate) fn now_secs(&self) -> u64 {
        self.now_secs
    }

    pub(crate) fn get_feed(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.feed_cache.get((adapter.clone(), feed_id.clone()))
    }

    pub(crate) fn set_feed(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.feed_cache
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Whether `key` is already on the resolve stack.
    pub(crate) fn is_resolving(&self, key: &PriceKey) -> bool {
        self.resolving_keys.iter().any(|k| k == *key)
    }

    /// Push `key` onto the cycle stack.
    ///
    /// # Errors
    /// * [`OracleError::OracleCycleDetected`] — `key` is already resolving.
    pub(crate) fn push_key(&mut self, key: &PriceKey) {
        if self.is_resolving(key) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving_keys.push_back(key.clone());
    }

    pub(crate) fn pop_key(&mut self) {
        self.resolving_keys.pop_back();
    }

    pub(crate) fn cached_price(&self, key: &PriceKey) -> Option<PriceFeedRaw> {
        self.key_prices.get(key.clone())
    }

    pub(crate) fn store_price(&mut self, key: &PriceKey, feed: PriceFeedRaw) {
        self.key_prices.set(key.clone(), feed);
    }

    pub(crate) fn cached_error(&self, key: &PriceKey) -> Option<OracleError> {
        self.key_errors.get(key.clone())
    }

    pub(crate) fn store_error(&mut self, key: &PriceKey, error: OracleError) {
        self.key_errors.set(key.clone(), error);
    }

    pub(crate) fn cached_status(&self, key: &PriceKey) -> Option<PriceStatus> {
        self.key_statuses.get(key.clone())
    }

    pub(crate) fn store_status(&mut self, key: &PriceKey, status: PriceStatus) {
        self.key_statuses.set(key.clone(), status);
    }

    /// Bulk-fetch multi-feed payloads for MultiFeed leaves under `keys`
    /// (including nested Scaled quotes). Best-effort: never reverts.
    ///
    /// Adapters with fewer than two distinct feed ids in this call are left
    /// for lazy single reads. Under `certora`, this is a no-op.
    #[cfg(feature = "certora")]
    pub(crate) fn warm(&mut self, _keys: &Vec<PriceKey>) {}

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

    let Some(oracle) = crate::admin::get_oracle(env, key) else {
        return;
    };

    for source in oracle.sources.iter() {
        match &source {
            PriceSource::Feed(feed) => collect_provider(env, by_adapter, &feed.provider),
            PriceSource::Scaled(scaled) => {
                collect_provider(env, by_adapter, &scaled.factor.provider);
                collect_key(env, by_adapter, visited, &scaled.quote, depth + 1);
            }
            // Refused at config validate; never stored successfully.
            PriceSource::LpShare(_) => {}
        }
    }
}

#[cfg(not(feature = "certora"))]
fn collect_provider(env: &Env, by_adapter: &mut Map<Address, Vec<String>>, provider: &ProviderRef) {
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

#[cfg(test)]
#[path = "../tests/oracle/context.rs"]
mod tests;
