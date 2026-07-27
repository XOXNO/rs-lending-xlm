//! Transaction-local resolution context: token-rooted feeds, multi-feed adapter
//! bulk cache, oracle-config memo, and the price-resolution cycle guard.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};


pub(crate) struct ResolutionContext {
    env: Env,
    /// Raw multi-feed adapter payloads (RedStone/Xoxno wire ABI) fetched once
    /// per transaction.
    bulk_feed_cache: Map<(Address, String), RedStonePriceData>,
    /// Keys whose properties or price are being derived right now — the
    /// resolution stack. A composition cycle recurses until this traps the
    /// re-entry and reverts with a clear error.
    resolving_keys: Vec<PriceKey>,
    /// Key-rooted USD prices resolved this transaction.
    ///
    /// Written by exactly one place - `engine::resolve`, after every guard has
    /// passed - so a cached entry is always a fully-checked one. A second
    /// writer that skipped a guard would serve an unchecked price to any later
    /// hard read that hit the same key.
    key_prices: Map<PriceKey, PriceFeedRaw>,
    /// Key-rooted diagnostic statuses resolved this transaction. Never read by
    /// the hard path.
    key_statuses: Map<PriceKey, PriceStatus>,
    current_timestamp_secs: u64,
}

impl ResolutionContext {
    /// Empty context for one transaction, pinned to the ledger timestamp read
    /// at construction.
    pub(crate) fn new(env: &Env) -> Self {
        ResolutionContext {
            env: env.clone(),
            bulk_feed_cache: Map::new(env),
            resolving_keys: Vec::new(env),
            key_prices: Map::new(env),
            key_statuses: Map::new(env),
            current_timestamp_secs: env.ledger().timestamp(),
        }
    }

    /// The `Env` this context was built from.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// The sampled ledger clock, so every freshness judgement in one
    /// transaction is made against the same instant.
    pub(crate) fn ledger_timestamp_secs(&self) -> u64 {
        self.current_timestamp_secs
    }

    /// Prefetched multi-feed adapter payload for `(adapter, feed_id)`, if any.
    pub(crate) fn get_bulk_feed(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.bulk_feed_cache.get((adapter.clone(), feed_id.clone()))
    }

    /// Stores a multi-feed adapter payload for the rest of the transaction.
    pub(crate) fn set_bulk_feed(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.bulk_feed_cache
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Marks `key` as being derived; reverts `OracleCycleDetected` if it is
    /// already on the stack.
    ///
    /// Pushed before the config is read, never after: a guard installed after
    /// resolution cannot see the re-entry it exists to catch.
    pub(crate) fn push_price_key(&mut self, key: &PriceKey) {
        if self.resolving_keys.iter().any(|k| k == *key) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving_keys.push_back(key.clone());
    }

    /// Pops the most recently entered key (caller ensures enter/exit balance).
    pub(crate) fn pop_price_key(&mut self) {
        self.resolving_keys.pop_back();
    }

    /// True when `key` is already on the resolution stack.
    ///
    /// The soft counterpart of [`Self::push_price_key`]: a diagnostic view must
    /// report a cycle as unusable rather than revert, so it probes first.
    pub(crate) fn is_price_key_resolving(&self, key: &PriceKey) -> bool {
        self.resolving_keys.iter().any(|k| k == *key)
    }

    /// Key-rooted diagnostic status resolved earlier this transaction, if any.
    ///
    /// Kept strictly separate from the price memo. A soft status is allowed to
    /// describe a price the hard path would reject, so letting the two share a
    /// map would be a way for an unusable reading to reach a fail-closed caller.
    pub(crate) fn cached_key_status(&self, key: &PriceKey) -> Option<PriceStatus> {
        self.key_statuses.get(key.clone())
    }

    pub(crate) fn store_key_status(&mut self, key: &PriceKey, status: PriceStatus) {
        self.key_statuses.set(key.clone(), status);
    }

    /// Key-rooted USD price resolved earlier this transaction, if any.
    pub(crate) fn cached_key_price(&self, key: &PriceKey) -> Option<PriceFeedRaw> {
        self.key_prices.get(key.clone())
    }

    /// Memoizes a fully-guarded key-rooted price for the rest of the
    /// transaction. See the field docs for why this must stay single-writer.
    pub(crate) fn store_key_price(&mut self, key: &PriceKey, feed: PriceFeedRaw) {
        self.key_prices.set(key.clone(), feed);
    }

}

#[cfg(test)]
#[path = "../tests/oracle/context.rs"]
mod tests;
