//! Transaction-local cache for oracle and market reads.
//!
//! Price and index reads are memoized per call. Position deltas buffer until
//! storage writes, then emit as one batch event.

use crate::constants::MS_PER_SECOND;
use crate::events::{
    EventBorrowDelta, EventDepositDelta, PositionAction, UpdatePositionBatchEvent,
};
use common::errors::{EModeError, OracleError};
use controller_interface::types::{
    Account, AccountPosition, DebtPosition, HubAssetKey, MarketIndex, MarketIndexRaw,
    MarketOracleConfig, PoolSyncData, PriceFeed, PriceFeedRaw, SpokeAssetConfig, SpokeConfig,
    SpokeUsageRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, String, Vec};

use crate::external::pool::{fetch_pool_bulk_indexes, fetch_pool_sync_data};
use crate::helpers::SpokeUsageContext;
use crate::oracle::token_price;
use crate::storage;
use common::oracle::providers::redstone::RedStonePriceData;

pub struct Cache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeedRaw>,
    /// Raw RedStone payloads bulk-fetched once per tx, keyed by (adapter, feed_id).
    /// Stores provider data, not resolved prices, so per-flow policy checks
    /// (staleness, sanity, tolerance) are unaffected.
    redstone_prefetch: Map<(Address, String), RedStonePriceData>,
    /// Borrow/supply indexes, populated only from the pool: either returned by a
    /// pool mutation (`put_market_index`) or bulk-read via `bulk_get_indexes`.
    /// The controller never simulates indexes itself.
    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,
    /// One loaded spoke per tx: usage buffer and cap writes.
    emode_usage: Option<SpokeUsageContext>,
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
            redstone_prefetch: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            emode_usage: None,
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

    pub fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        (&token_price(self, asset)).into()
    }

    pub fn get_redstone_prefetch(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.redstone_prefetch
            .get((adapter.clone(), feed_id.clone()))
    }

    pub fn set_redstone_prefetch(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.redstone_prefetch
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Token-rooted oracle config under `AssetOracle(asset)`. Panics
    /// `OracleNotConfigured` when absent. Absence is the pending/disabled gate:
    /// price resolution reverts for any asset with no `AssetOracle` entry.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> MarketOracleConfig {
        storage::get_asset_oracle(&self.env, asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }

    /// Oracle config for `asset`: the token-rooted `AssetOracle` base. Pricing is
    /// hub-independent and keyed by the bare asset; there is no per-spoke override
    /// resolution in this path (a hub-correct override would require the asset's
    /// real hub, which the token-rooted price path does not carry).
    pub(crate) fn resolve_oracle_config(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle(asset)
    }

    /// Address of the central liquidity pool, memoized for the transaction.
    pub fn cached_pool_address(&mut self) -> Address {
        if let Some(addr) = &self.pool_address {
            return addr.clone();
        }
        let addr = storage::get_pool(&self.env);
        self.pool_address = Some(addr.clone());
        addr
    }

    /// Caches an index the pool returned from a mutation (`PoolPositionMutation.
    /// market_index`). Lets the post-action valuation skip a redundant pool read
    /// for the touched hub-asset.
    pub fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// Certora stub: lazy per-asset reads preserve semantics.
    #[cfg(feature = "certora")]
    pub fn prefetch_market_indexes(&mut self, _hub_assets: &Vec<HubAssetKey>) {}

    /// Seeds `market_indexes` for uncached hub-assets.
    /// Skips duplicates and markets already loaded in this transaction. The pool
    /// reverts `PoolNotInitialized` for any uncreated (hub, asset) in the batch.
    #[cfg(not(feature = "certora"))]
    pub fn prefetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let mut missing: Vec<HubAssetKey> = Vec::new(&self.env);
        for hub_asset in hub_assets.iter() {
            if self.market_indexes.contains_key(hub_asset.clone())
                || missing.first_index_of(hub_asset.clone()).is_some()
            {
                continue;
            }
            missing.push_back(hub_asset);
        }
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

    /// Returns the pool-sourced index for `hub_asset`. On a cache miss the pool is
    /// asked for it (single-asset `bulk_get_indexes`); the controller never
    /// simulates accrual itself.
    pub fn cached_market_index(&mut self, hub_asset: &HubAssetKey) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(hub_asset.clone()) {
            return (&index).into();
        }
        let pool_addr = self.cached_pool_address();
        let mut request = Vec::new(&self.env);
        request.push_back(hub_asset.clone());
        let index = fetch_pool_bulk_indexes(&self.env, &pool_addr, &request).get_unchecked(0);
        self.market_indexes.set(hub_asset.clone(), index.clone());
        (&index).into()
    }

    pub fn record_position_update(
        &mut self,
        action: PositionAction,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.deposit_updates.push_back(EventDepositDelta::new(
            action,
            asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    pub fn record_debt_position_update(
        &mut self,
        action: PositionAction,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.borrow_updates.push_back(EventBorrowDelta::new(
            action,
            asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    pub fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.deposit_updates.is_empty() && self.borrow_updates.is_empty() {
            return;
        }
        UpdatePositionBatchEvent {
            account_id,
            account_attributes: account.into(),
            deposits: self.deposit_updates.clone(),
            borrows: self.borrow_updates.clone(),
        }
        .publish(&self.env);
        self.deposit_updates = Vec::new(&self.env);
        self.borrow_updates = Vec::new(&self.env);
    }

    pub fn cached_pool_sync_data(&mut self, hub_asset: &HubAssetKey) -> PoolSyncData {
        if let Some(data) = self.pool_sync_data.get(hub_asset.clone()) {
            return data;
        }
        let pool_addr = self.cached_pool_address();
        let data = fetch_pool_sync_data(&self.env, &pool_addr, hub_asset);
        self.pool_sync_data.set(hub_asset.clone(), data.clone());
        data
    }

    /// Loads the account's spoke once per transaction when first needed. Every
    /// account binds to a real spoke (id `>= 1`), so this always loads a context.
    pub(crate) fn ensure_spoke_loaded(&mut self, spoke_id: u32) {
        if let Some(ctx) = &self.emode_usage {
            assert_with_error!(
                &self.env,
                ctx.spoke_id() == spoke_id,
                EModeError::EModeMismatch
            );
            return;
        }
        self.emode_usage = SpokeUsageContext::load(&self.env, spoke_id);
    }

    pub fn cached_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.emode_usage
            .as_ref()
            .and_then(|ctx| ctx.spoke_asset(&env, hub_asset))
    }

    pub fn cached_spoke(&mut self, spoke_id: u32) -> Option<SpokeConfig> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.emode_usage.as_ref().map(|ctx| ctx.as_spoke(&env))
    }

    pub fn active_spoke(&mut self, env: &Env, spoke_id: u32) -> Option<SpokeConfig> {
        let spoke = self.cached_spoke(spoke_id)?;
        crate::emode::ensure_spoke_not_deprecated(env, &Some(spoke.clone()));
        Some(spoke)
    }

    pub fn cached_spoke_usage(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeUsageRaw> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.emode_usage
            .as_mut()
            .map(|ctx| ctx.spoke_usage(&env, hub_asset))
    }

    pub(crate) fn spoke_usage_mut(&mut self, spoke_id: u32) -> Option<&mut SpokeUsageContext> {
        self.ensure_spoke_loaded(spoke_id);
        self.emode_usage.as_mut()
    }

    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.emode_usage {
            ctx.persist(&self.env);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/cache/resolve.rs"]
mod tests;
