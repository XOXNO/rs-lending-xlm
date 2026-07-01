//! Transaction-local cache for oracle and market reads.

mod events;
mod market_index;
mod oracle;
mod pool;
mod spoke;

use crate::constants::MS_PER_SECOND;
use crate::events::{EventBorrowDelta, EventDepositDelta};
use common::oracle::providers::redstone::RedStonePriceData;
#[cfg(test)]
use common::types::SpokeAssetConfig;
use common::types::{HubAssetKey, MarketIndexRaw, MarketOracleConfig, PoolSyncData, PriceFeedRaw};
use soroban_sdk::{Address, Env, Map, String, Vec};

use crate::spoke::SpokeUsageContext;
use crate::storage;

pub struct Cache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeedRaw>,
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
    /// One loaded spoke per tx: usage buffer and cap writes.
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

    pub(crate) fn build(env: &Env) -> Self {
        let current_timestamp_ms = env.ledger().timestamp() * MS_PER_SECOND;

        Cache {
            env: env.clone(),
            prices_cache: Map::new(env),
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

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Ledger timestamp in whole seconds (derived from `current_timestamp_ms`).
    pub fn ledger_timestamp_secs(&self) -> u64 {
        self.current_timestamp_ms / MS_PER_SECOND
    }
}

#[cfg(test)]
#[path = "../../tests/cache/resolve.rs"]
mod tests;
