//! Invocation-local market data, spoke usage, and ordered position-event deltas.
//! Reuse cached values within a flow; persist usage and drain events explicitly.
//! Certora replaces bulk index fetching through `spec_hooks.rs`.

use common::collections::{collect_uncached_keys, unique_hub_tokens};
use common::errors::{GenericError, OracleError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    Account, AccountPosition, AssetConfig, DebtPosition, HubAssetKey, MarketIndex, MarketIndexRaw,
    PoolSyncData, PriceFeed, PriceFeedRaw, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::{assert_with_error, panic_with_error, vec, Address, Env, Map, Vec};

use crate::config::require_hub_active;
use crate::events::{
    EventBorrowDelta, EventDepositDelta, PositionAction, UpdatePositionBatchEvent,
};
use crate::external::{
    self,
    pool::{fetch_pool_bulk_indexes, fetch_pool_sync_data},
};
use crate::spoke_usage::{SpokeUsageContext, UsageSide};
use crate::storage;

pub(crate) struct Context {
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

impl Context {
    /// Creates an empty context and renews controller instance TTL.
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::new_view(env)
    }

    /// Creates an empty context without renewing instance TTL.
    pub(crate) fn new_view(env: &Env) -> Self {
        Context {
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

    /// Returns this context's environment.
    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    /// Loads missing prices by token address and missing indexes by hub-asset key.
    /// Already cached values are retained.
    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }

    /// Requires an active hub, caching successful checks for this invocation.
    pub(crate) fn require_hub_active(&mut self, hub_id: u32) {
        if self.verified_hubs.contains_key(hub_id) {
            return;
        }
        require_hub_active(&self.env, hub_id);
        self.verified_hubs.set(hub_id, true);
    }

    /// Returns the pool address, loading it from storage on first access.
    pub(crate) fn cached_pool_address(&mut self) -> Address {
        self.pool_address
            .get_or_insert_with(|| storage::get_pool(&self.env))
            .clone()
    }

    /// Returns stored pool parameters and state, fetching once per hub asset.
    /// Does not accrue interest or refresh a cached snapshot.
    pub(crate) fn cached_pool_sync_data(&mut self, hub_asset: &HubAssetKey) -> PoolSyncData {
        if let Some(data) = self.pool_sync_data.get(hub_asset.clone()) {
            return data;
        }
        let pool_addr = self.cached_pool_address();
        let data = fetch_pool_sync_data(&self.env, &pool_addr, hub_asset);
        self.pool_sync_data.set(hub_asset.clone(), data.clone());
        data
    }

    /// Replaces the cached index with a pool mutation's updated index.
    pub(crate) fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// Fetches missing simulated indexes in one pool call; retains cached indexes.
    #[cfg(not(feature = "certora"))]
    pub(crate) fn fetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let missing = collect_uncached_keys(&self.env, hub_assets, &self.market_indexes);
        if missing.is_empty() {
            return;
        }
        let pool_addr = self.cached_pool_address();
        let indexes = fetch_pool_bulk_indexes(&self.env, &pool_addr, &missing);
        for (i, hub_asset) in missing.iter().enumerate() {
            self.market_indexes
                .set(hub_asset, indexes.get_unchecked(i as u32));
        }
    }

    /// Returns a cached index or fetches its simulated current value from the pool.
    pub(crate) fn cached_market_index(&mut self, hub_asset: &HubAssetKey) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(hub_asset.clone()) {
            return (&index).into();
        }
        let pool_addr = self.cached_pool_address();
        let request = vec![&self.env, hub_asset.clone()];
        let index = fetch_pool_bulk_indexes(&self.env, &pool_addr, &request).get_unchecked(0);
        self.market_indexes.set(hub_asset.clone(), index.clone());
        (&index).into()
    }

    #[cfg(test)]
    pub(crate) fn set_prices(&mut self, prices: Map<Address, PriceFeedRaw>) {
        self.token_prices = prices;
    }

    /// Fetches missing prices in one aggregator call; retains cached prices.
    pub(crate) fn fetch_prices(&mut self, assets: &Vec<Address>) {
        let missing = collect_uncached_keys(&self.env, assets, &self.token_prices);
        if missing.is_empty() {
            return;
        }
        let fetched = external::price_aggregator::fetch_prices(&self.env, &missing);
        for (asset, feed) in fetched.iter() {
            self.token_prices.set(asset, feed);
        }
    }

    /// Returns a previously loaded price; fails if the cache has no entry.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }

    /// Binds usage to one spoke; rejects another spoke until the context is reset.
    fn ensure_spoke_context(&mut self, spoke_id: u32) {
        if let Some(ctx) = &self.spoke_usage {
            assert_with_error!(
                &self.env,
                ctx.spoke_id() == spoke_id,
                SpokeError::SpokeMismatch
            );
            return;
        }
        self.spoke_usage = Some(SpokeUsageContext::new(&self.env, spoke_id));
    }

    /// Discards spoke usage and configuration caches without persisting them.
    /// Retains market data, hub checks, and event buffers.
    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
        self.spoke_config = None;
        self.spoke_assets = Map::new(&self.env);
    }

    /// Loads usage for this spoke; rejects a different already-loaded spoke.
    fn require_spoke_usage_context(&mut self, spoke_id: u32) -> &mut SpokeUsageContext {
        self.ensure_spoke_context(spoke_id);
        self.spoke_usage
            .as_mut()
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError))
    }

    /// Returns the listed spoke asset config, caching successful reads only.
    pub(crate) fn cached_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        self.ensure_spoke_context(spoke_id);
        if let Some(cfg) = self.spoke_assets.get(hub_asset.clone()) {
            return Some(cfg);
        }
        let loaded = storage::get_spoke_asset(&self.env, spoke_id, hub_asset)?;
        self.spoke_assets.set(hub_asset.clone(), loaded.clone());
        Some(loaded)
    }

    /// Returns risk parameters for a listed asset; fails if it is unlisted.
    pub(crate) fn require_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        let asset = self.require_spoke_asset_config(spoke_id, hub_asset);

        (&asset).into()
    }

    /// Returns the full listed asset config; fails if it is unlisted.
    pub(crate) fn require_spoke_asset_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> SpokeAssetConfig {
        self.cached_spoke_asset(spoke_id, hub_asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, SpokeError::AssetNotInSpoke))
    }

    /// Returns listed risk parameters after rejecting a deprecated spoke.
    pub(crate) fn require_listed_active_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        self.active_spoke(spoke_id);
        self.require_spoke_asset(spoke_id, hub_asset)
    }

    /// Returns the spoke config, loading it once until the spoke context is reset.
    pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        self.ensure_spoke_context(spoke_id);
        self.spoke_config
            .get_or_insert_with(|| storage::get_spoke(&self.env, spoke_id))
            .clone()
    }

    /// Returns the spoke config, rejecting deprecated spokes.
    pub(crate) fn active_spoke(&mut self, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        assert_with_error!(&self.env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
        spoke
    }

    /// Buffers a scaled usage increase after checking this side's cap at `market_index`.
    pub(crate) fn apply_spoke_entry(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let spoke_config = self.require_spoke_asset_config(spoke_id, hub_asset);
        let cap = side.cap(&spoke_config);
        let index = side.index(market_index);
        self.require_spoke_usage_context(spoke_id).apply_entry(
            side,
            hub_asset,
            delta_scaled,
            cap,
            index,
            decimals,
        );
    }

    /// Buffers a scaled usage decrease; missing usage is a no-op, underflow fails.
    pub(crate) fn apply_spoke_exit(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        self.require_spoke_usage_context(spoke_id)
            .apply_exit(side, hub_asset, delta_scaled);
    }

    /// Writes cached spoke usage without clearing it; no-op if none is loaded.
    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist();
        }
    }

    /// Appends a supply delta for the next position batch, preserving insertion order.
    pub(crate) fn record_supply_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.supply_updates.push_back(EventDepositDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Appends a debt delta for the next position batch, preserving insertion order.
    pub(crate) fn record_debt_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.debt_updates.push_back(EventBorrowDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Publishes buffered deltas for `account_id` and clears only the event buffers.
    /// Emits nothing when both buffers are empty.
    pub(crate) fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.supply_updates.is_empty() && self.debt_updates.is_empty() {
            return;
        }
        UpdatePositionBatchEvent {
            account_id,
            account_attributes: account.into(),
            deposits: self.supply_updates.clone(),
            borrows: self.debt_updates.clone(),
        }
        .publish(&self.env);
        self.supply_updates = Vec::new(&self.env);
        self.debt_updates = Vec::new(&self.env);
    }
}

#[cfg(test)]
#[path = "../tests/context/oracle.rs"]
mod tests;
