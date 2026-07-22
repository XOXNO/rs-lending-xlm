//! Transaction-local resolution context: token-rooted feeds, multi-feed adapter
//! bulk cache, oracle-config memo, and the price-resolution cycle guard.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{AssetOracleConfig, PriceFeedRaw};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

use crate::storage;

pub(crate) struct ResolutionContext {
    env: Env,
    /// Token-rooted USD price feeds resolved this transaction.
    token_prices: Map<Address, PriceFeedRaw>,
    /// Assets whose USD price is being resolved right now (the resolution stack).
    /// A quote/anchor cycle (A quoted in B, B quoted in A) recurses until this
    /// shadow stack traps the re-entry and reverts with a clear error.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    resolving: Vec<Address>,
    /// Raw multi-feed adapter payloads (RedStone/Xoxno wire ABI) fetched once
    /// per transaction.
    bulk_feed_cache: Map<(Address, String), RedStonePriceData>,
    /// Token-rooted oracle configs; absence is not memoized (repeated probes
    /// re-hit storage until configured).
    asset_oracle: Map<Address, AssetOracleConfig>,
    current_timestamp_secs: u64,
}

impl ResolutionContext {
    pub(crate) fn new(env: &Env) -> Self {
        ResolutionContext {
            env: env.clone(),
            token_prices: Map::new(env),
            resolving: Vec::new(env),
            bulk_feed_cache: Map::new(env),
            asset_oracle: Map::new(env),
            current_timestamp_secs: env.ledger().timestamp(),
        }
    }

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    pub(crate) fn ledger_timestamp_secs(&self) -> u64 {
        self.current_timestamp_secs
    }

    /// USD price feed resolved earlier this transaction, if any.
    pub(crate) fn cached_price(&self, asset: &Address) -> Option<PriceFeedRaw> {
        self.token_prices.get(asset.clone())
    }

    /// Only the (certora-stubbed) prefetch pass needs the existence probe.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn has_price(&self, asset: &Address) -> bool {
        self.token_prices.contains_key(asset.clone())
    }

    pub(crate) fn store_price(&mut self, asset: &Address, feed: PriceFeedRaw) {
        self.token_prices.set(asset.clone(), feed);
    }

    /// Marks `asset` as being priced; reverts `OracleCycleDetected` if it is
    /// already on the stack. Must pair with `pop_resolution` on success.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn push_resolution(&mut self, asset: &Address) {
        if self.resolving.iter().any(|a| a == *asset) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving.push_back(asset.clone());
    }

    /// Pops the most recently entered asset (caller ensures enter/exit balance).
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn pop_resolution(&mut self) {
        self.resolving.pop_back();
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

    /// Token-rooted oracle config if configured (absence not memoized).
    pub(crate) fn cached_asset_oracle_opt(&mut self, asset: &Address) -> Option<AssetOracleConfig> {
        if let Some(config) = self.asset_oracle.get(asset.clone()) {
            return Some(config);
        }
        let config = storage::get_oracle_config(&self.env, asset)?;
        self.asset_oracle.set(asset.clone(), config.clone());
        Some(config)
    }

    /// Required token-rooted oracle config, or `OracleNotConfigured`.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> AssetOracleConfig {
        self.cached_asset_oracle_opt(asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }
}

#[cfg(test)]
#[path = "../tests/oracle/context.rs"]
mod tests;
