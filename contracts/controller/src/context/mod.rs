//! Per-invocation `Cache`: memoizes reads (prices, market indexes, spoke
//! config and assets, hub checks) and buffers writes (spoke usage deltas and
//! the position-event queue drained by `emit_position_batch`). `new` renews
//! the instance TTL; `new_view` serves read-only entry points. The impl spans
//! the sibling files, one cache concern each; under the certora feature,
//! `spec_hooks.rs` overrides index fetching.

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

use crate::spoke_usage::SpokeUsageContext;
use crate::storage;

pub(crate) struct Cache {
    env: Env,

    token_prices: Map<Address, PriceFeedRaw>,

    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,

    spoke_usage: Option<SpokeUsageContext>,

    spoke_config: Option<SpokeConfig>,

    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,

    verified_hubs: Map<u32, bool>,

    supply_updates: Vec<EventDepositDelta>,

    debt_updates: Vec<EventBorrowDelta>,
}

impl Cache {
    /// Renews the controller's instance storage TTL and returns a fresh, empty cache for a state-changing entrypoint.
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env)
    }

    /// Returns a fresh, empty cache for a read-only entrypoint, without renewing the instance storage TTL.
    pub(crate) fn new_view(env: &Env) -> Self {
        Self::build(env)
    }

    /// Constructs an empty `Cache` with all memoization maps and update buffers initialized but unpopulated.
    fn build(env: &Env) -> Self {
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

    /// Returns the cached `Env` handle.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Deduplicates the token addresses referenced by `hub_assets`, then fetches and caches any of their prices and market indexes not already cached.
    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }

    /// Verifies hub `hub_id` is active, panicking otherwise, and memoizes the result so repeated calls for the same hub skip the check.
    pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
        if self.verified_hubs.contains_key(hub_id) {
            return;
        }
        crate::config::require_hub_active(&self.env, hub_id);
        self.verified_hubs.set(hub_id, true);
    }
}
