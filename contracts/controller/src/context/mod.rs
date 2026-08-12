//! Per-invocation cache that memoizes prices, market indexes, pool and
//! spoke configuration, and hub-active checks read during a controller
//! call, and buffers position-update events for batched publishing.

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

/// Per-invocation cache of controller state: token prices, market indexes,
/// pool address and sync data, spoke usage and configuration, verified hub
/// ids, and pending position-update events. Populated lazily as accessor
/// methods on `Cache` (defined in the sibling modules) are called, and
/// discarded at the end of the invocation.
pub(crate) struct Cache {
    env: Env,

    token_prices: Map<Address, PriceFeedRaw>,

    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,

    spoke_usage: Option<SpokeUsageContext>,

    spoke_config: Option<SpokeConfig>,

    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,

    /// Hub ids already proven active this invocation; see [`Cache::require_hub_active`].
    verified_hubs: Map<u32, bool>,

    supply_updates: Vec<EventDepositDelta>,

    debt_updates: Vec<EventBorrowDelta>,
}

impl Cache {
    /// Renews the controller instance's storage TTL, then builds a fresh,
    /// empty `Cache`. Use for state-mutating invocations.
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env)
    }

    /// Builds a fresh, empty `Cache` without renewing the controller
    /// instance's storage TTL. Use for read-only invocations.
    pub(crate) fn new_view(env: &Env) -> Self {
        Self::build(env)
    }

    /// Constructs a `Cache` with all maps and buffers empty and no cached
    /// pool address, spoke usage, or spoke configuration.
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
            verified_hubs: Map::new(env),
            supply_updates: Vec::new(env),
            debt_updates: Vec::new(env),
        }
    }

    /// Returns the cached `Env`.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Loads and caches the price and market index data needed for
    /// `hub_assets`. Derives the set of unique underlying tokens from
    /// `hub_assets` and fetches their prices, then fetches market indexes
    /// for `hub_assets` directly.
    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }

    /// Verifies that `hub_id` is active, memoizing the result.
    pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
        if self.verified_hubs.contains_key(hub_id) {
            return;
        }
        crate::config::require_hub_active(&self.env, hub_id);
        self.verified_hubs.set(hub_id, true);
    }
}
