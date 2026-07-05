//! Transaction-local cache for oracle/market/pool reads, plus a write buffer
//! for spoke usage and position events pending emission.

mod events;
mod market_index;
mod oracle;
mod pool;
mod spoke;

use crate::constants::MS_PER_SECOND;
use crate::events::{EventBorrowDelta, EventDepositDelta};
use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
#[cfg(test)]
use common::types::SpokeAssetConfig;
use common::types::{HubAssetKey, MarketIndexRaw, MarketOracleConfig, PoolSyncData, PriceFeedRaw};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

use crate::spoke::SpokeUsageContext;
use crate::storage;

pub struct Cache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeedRaw>,
    /// Assets whose USD price is being resolved right now (the resolution stack).
    /// `token_price` writes `prices_cache` only after fully resolving, so a
    /// quote/anchor cycle (A quoted in B, B quoted in A) would recurse until the
    /// shadow stack traps. Membership here detects the re-entry and reverts with
    /// a clear error instead.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    resolving: Vec<Address>,
    /// Per-spoke override price cache, separate from token-rooted prices.
    spoke_prices: Map<HubAssetKey, PriceFeedRaw>,
    /// Raw RedStone payloads fetched once per transaction.
    redstone_prefetch: Map<(Address, String), RedStonePriceData>,
    /// Token-rooted oracle configs; missing entries are not cached.
    asset_oracle: Map<Address, MarketOracleConfig>,
    /// Pool-sourced borrow/supply indexes; controller never simulates accrual.
    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,
    /// One loaded spoke at a time: usage buffer and cap writes. Reset between
    /// accounts (`reset_spoke_context`) so one batch can cover several spokes.
    spoke_usage: Option<SpokeUsageContext>,
    deposit_updates: Vec<EventDepositDelta>,
    borrow_updates: Vec<EventBorrowDelta>,

    pub current_timestamp_ms: u64,
}

impl Cache {
    /// Creates a cache for mutating flows and renews controller instance TTL.
    pub fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env)
    }

    /// Creates a read-only cache that does not renew instance TTL.
    pub fn new_view(env: &Env) -> Self {
        Self::build(env)
    }

    /// Builds a cache with empty per-transaction memos seeded from the current ledger timestamp.
    pub(crate) fn build(env: &Env) -> Self {
        let current_timestamp_ms = env.ledger().timestamp() * MS_PER_SECOND;

        Cache {
            env: env.clone(),
            prices_cache: Map::new(env),
            resolving: Vec::new(env),
            spoke_prices: Map::new(env),
            redstone_prefetch: Map::new(env),
            asset_oracle: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            spoke_usage: None,
            deposit_updates: Vec::new(env),
            borrow_updates: Vec::new(env),
            current_timestamp_ms,
        }
    }

    /// Returns the transaction environment handle.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Ledger timestamp in whole seconds (derived from `current_timestamp_ms`).
    pub fn ledger_timestamp_secs(&self) -> u64 {
        self.current_timestamp_ms / MS_PER_SECOND
    }

    /// Marks `asset` as being priced and reverts `OracleCycleDetected` if it is
    /// already on the resolution stack — a quote/anchor cycle that would
    /// otherwise recurse until the shadow stack traps. Paired with
    /// `exit_price_resolution` on the success path.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn enter_price_resolution(&mut self, asset: &Address) {
        if self.resolving.iter().any(|a| a == *asset) {
            panic_with_error!(&self.env, OracleError::OracleCycleDetected);
        }
        self.resolving.push_back(asset.clone());
    }

    /// Pops the most recently entered asset off the resolution stack.
    #[cfg_attr(feature = "certora", allow(dead_code))]
    pub(crate) fn exit_price_resolution(&mut self) {
        self.resolving.pop_back();
    }
}

#[cfg(test)]
#[path = "../../tests/cache/resolve.rs"]
mod tests;
