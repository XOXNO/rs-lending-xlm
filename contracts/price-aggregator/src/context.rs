//! Transaction-local price cache: token-rooted feeds, RedStone prefetch,
//! oracle-config memo, and the price-resolution cycle guard.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{MarketOracleConfig, PriceFeedRaw};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

use crate::storage;

pub(crate) struct PriceCache {
    env: Env,
    /// Token-rooted USD price feeds resolved this transaction.
    pub(crate) token_prices: Map<Address, PriceFeedRaw>,
    /// Assets whose USD price is being resolved right now (the resolution stack).
    /// A quote/anchor cycle (A quoted in B, B quoted in A) recurses until this
    /// shadow stack traps the re-entry and reverts with a clear error.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    resolving: Vec<Address>,
    /// Raw RedStone payloads fetched once per transaction.
    redstone_prefetch: Map<(Address, String), RedStonePriceData>,
    /// Token-rooted oracle configs; absence is not memoized (repeated probes
    /// re-hit storage until configured).
    asset_oracle: Map<Address, MarketOracleConfig>,
    current_timestamp_secs: u64,
}

impl PriceCache {
    pub(crate) fn new(env: &Env) -> Self {
        Self::build(env)
    }

    /// Read-only cache; identical to `new` (the aggregator holds no instance TTL).
    pub(crate) fn new_view(env: &Env) -> Self {
        Self::build(env)
    }

    fn build(env: &Env) -> Self {
        PriceCache {
            env: env.clone(),
            token_prices: Map::new(env),
            resolving: Vec::new(env),
            redstone_prefetch: Map::new(env),
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

    /// Marks `asset` as being priced; reverts `OracleCycleDetected` if it is
    /// already on the stack. Must pair with `exit_price_resolution` on success.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn enter_price_resolution(&mut self, asset: &Address) {
        if self.resolving.iter().any(|a| a == *asset) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving.push_back(asset.clone());
    }

    /// Pops the most recently entered asset (caller ensures enter/exit balance).
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn exit_price_resolution(&mut self) {
        self.resolving.pop_back();
    }

    /// Prefetched RedStone payload for `(adapter, feed_id)`, if any.
    pub(crate) fn get_redstone_prefetch(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.redstone_prefetch
            .get((adapter.clone(), feed_id.clone()))
    }

    /// Stores a RedStone payload for the rest of the transaction.
    pub(crate) fn set_redstone_prefetch(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.redstone_prefetch
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Token-rooted oracle config if configured (absence not memoized).
    pub(crate) fn cached_asset_oracle_opt(
        &mut self,
        asset: &Address,
    ) -> Option<MarketOracleConfig> {
        if let Some(config) = self.asset_oracle.get(asset.clone()) {
            return Some(config);
        }
        let config = storage::get_asset_oracle(&self.env, asset)?;
        self.asset_oracle.set(asset.clone(), config.clone());
        Some(config)
    }

    /// Required token-rooted oracle config, or `OracleNotConfigured`.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle_opt(asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }
}
