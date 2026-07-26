//! Transaction-local cache for oracle/market/pool reads, plus write buffers for
//! spoke usage and position events.
//!
//! # Lifecycle (call-site contract)
//! 1. Mutating flows: `Cache::new` (renews instance TTL). Views: `new_view`.
//! 2. Reads memoize on first use; pool indexes come only from the pool (or
//!    `put_market_index` after a mutation return).
//! 3. One spoke at a time in the usage buffer; multi-account batches
//!    `persist_spoke_usage` then `reset_spoke_context` before the next spoke.
//! 4. Flow tail: `persist_spoke_usage` → position storage → `emit_position_batch`
//!    (see `finalize_position_flow` and liquidation's custom persist path).

mod events;
mod market_index;
mod oracle;
mod pool;
mod spoke;

use crate::events::{EventBorrowDelta, EventDepositDelta};
use common::collections::unique_hub_tokens;
use common::types::{
    HubAssetKey, MarketIndexRaw, PoolSyncData, PriceFeedRaw, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::{Address, Env, Map, Vec};

use crate::spoke::SpokeUsageContext;
use crate::storage;

pub(crate) struct Cache {
    env: Env,

    /// Token-rooted USD prices for the current flow, filled by
    /// [`Self::fetch_prices`] / [`Self::load_markets`], then read as map lookups.
    pub(crate) token_prices: Map<Address, PriceFeedRaw>,
    /// Pool-sourced borrow/supply indexes; controller never simulates accrual.
    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,
    /// One loaded spoke at a time: usage buffer and cap writes. Reset between
    /// accounts (`reset_spoke_context`) so one batch can cover several spokes.
    spoke_usage: Option<SpokeUsageContext>,
    /// Config of the bound spoke, loaded once. Memoized because the Cache is
    /// fresh per transaction and no governance write to `Spoke` happens inside
    /// a user flow. Cleared with the usage buffer on `reset_spoke_context`.
    spoke_config: Option<SpokeConfig>,
    /// Per-asset listings for the bound spoke. A missing entry is never
    /// memoized, so an unlisted asset still reverts on every touch.
    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,
    /// Supply-side position event deltas (supply, withdraw, liq seize, …).
    supply_updates: Vec<EventDepositDelta>,
    /// Debt-side position event deltas (borrow, repay, liq repay, …).
    debt_updates: Vec<EventBorrowDelta>,
}

impl Cache {
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env)
    }

    /// Read-only cache (no instance TTL renew).
    pub(crate) fn new_view(env: &Env) -> Self {
        Self::build(env)
    }

    pub(crate) fn build(env: &Env) -> Self {
        Cache {
            env: env.clone(),
            token_prices: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            spoke_usage: None,
            spoke_config: None,
            spoke_assets: Map::new(env),
            supply_updates: Vec::new(env),
            debt_updates: Vec::new(env),
        }
    }

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Bulk-fetch USD prices and pool market indexes for hub-asset markets.
    ///
    /// Idempotent within a transaction: already-cached entries are skipped.
    /// Token addresses are deduped (same asset on multiple hubs or both sides
    /// of the book). Token-only pricing uses [`Self::fetch_prices`] instead.
    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }
}
