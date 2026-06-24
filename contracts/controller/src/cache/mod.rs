//! Transaction-local cache for oracle policy and market reads.
//!
//! Each mutating entrypoint creates a cache with its `OraclePolicy`. Price
//! and index reads follow that policy for the call. Position deltas buffer
//! until storage writes, then emit as one batch event.

use crate::constants::MS_PER_SECOND;
use crate::events::{
    EventBorrowDelta, EventDepositDelta, PositionAction, UpdatePositionBatchEvent,
};
use common::errors::EModeError;
use controller_interface::types::{
    Account, AccountPosition, AssetConfig, DebtPosition, EModeAssetConfig, EModeCategory,
    EModeSpokeUsageRaw, MarketConfig, MarketIndex, MarketIndexRaw, PoolSyncData, PriceFeed,
    PriceFeedRaw,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, String, Vec};

use crate::external::pool::{fetch_pool_bulk_indexes, fetch_pool_sync_data};
use crate::helpers::EModeUsageContext;
use crate::oracle::policy::OraclePolicy;
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
    pub market_configs: Map<Address, MarketConfig>,
    /// Borrow/supply indexes, populated only from the pool: either returned by a
    /// pool mutation (`put_market_index`) or bulk-read via `bulk_get_indexes`.
    /// The controller never simulates indexes itself.
    market_indexes: Map<Address, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<Address, PoolSyncData>,
    /// One loaded category per tx: spoke configs, usage totals, and cap writes.
    emode_usage: Option<EModeUsageContext>,
    deposit_updates: Vec<EventDepositDelta>,
    borrow_updates: Vec<EventBorrowDelta>,

    pub current_timestamp_ms: u64,
    pub oracle_policy: OraclePolicy,
}

impl Cache {
    /// Creates a cache for mutating flows and renews controller instance TTL.
    pub fn new(env: &Env, oracle_policy: OraclePolicy) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env, oracle_policy)
    }

    /// Creates a read-only cache with permissive view oracle policy.
    pub fn new_view(env: &Env) -> Self {
        Self::build(env, OraclePolicy::View)
    }

    pub(crate) fn build(env: &Env, oracle_policy: OraclePolicy) -> Self {
        let current_timestamp_ms = env.ledger().timestamp() * MS_PER_SECOND;

        Cache {
            env: env.clone(),
            prices_cache: Map::new(env),
            redstone_prefetch: Map::new(env),
            market_configs: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            emode_usage: None,
            deposit_updates: Vec::new(env),
            borrow_updates: Vec::new(env),
            current_timestamp_ms,
            oracle_policy,
        }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Drops resolved oracle feeds so the next read re-runs policy checks.
    pub(crate) fn clear_resolved_prices(&mut self) {
        self.prices_cache = Map::new(&self.env);
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

    pub fn cached_market_config(&mut self, asset: &Address) -> MarketConfig {
        self.try_cached_market_config(asset).unwrap_or_else(|| {
            panic_with_error!(&self.env, common::errors::GenericError::AssetNotSupported)
        })
    }

    /// Like [`Self::cached_market_config`], but returns `None` for assets
    /// with no configured market instead of panicking.
    pub fn try_cached_market_config(&mut self, asset: &Address) -> Option<MarketConfig> {
        if let Some(config) = self.market_configs.get(asset.clone()) {
            return Some(config);
        }
        let config = storage::try_get_market_config(&self.env, asset)?;
        self.market_configs.set(asset.clone(), config.clone());
        Some(config)
    }

    pub fn cached_asset_config(&mut self, asset: &Address) -> AssetConfig {
        (&self.cached_market_config(asset).asset_config).into()
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
    /// for the touched asset.
    pub fn put_market_index(&mut self, asset: &Address, index: &MarketIndexRaw) {
        self.market_indexes.set(asset.clone(), index.clone());
    }

    /// Certora stub: lazy per-asset reads preserve semantics.
    #[cfg(feature = "certora")]
    pub fn prefetch_market_indexes(&mut self, _assets: &Vec<Address>) {}

    /// Seeds `market_indexes` for listed, uncached assets.
    /// Skips duplicates and assets already loaded in this transaction.
    #[cfg(not(feature = "certora"))]
    pub fn prefetch_market_indexes(&mut self, assets: &Vec<Address>) {
        let mut missing: Vec<Address> = Vec::new(&self.env);
        for asset in assets.iter() {
            if self.market_indexes.contains_key(asset.clone())
                || missing.first_index_of(asset.clone()).is_some()
                || self.try_cached_market_config(&asset).is_none()
            {
                continue;
            }
            missing.push_back(asset);
        }
        if missing.is_empty() {
            return;
        }
        let pool_addr = self.cached_pool_address();
        let indexes = fetch_pool_bulk_indexes(&self.env, &pool_addr, &missing);
        for (i, asset) in missing.iter().enumerate() {
            self.market_indexes
                .set(asset, indexes.get_unchecked(i as u32));
        }
    }

    /// Returns the pool-sourced index for `asset`. On a cache miss the pool is
    /// asked for it (single-asset `bulk_get_indexes`); the controller never
    /// simulates accrual itself.
    pub fn cached_market_index(&mut self, asset: &Address) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(asset.clone()) {
            return (&index).into();
        }
        let pool_addr = self.cached_pool_address();
        let mut request = Vec::new(&self.env);
        request.push_back(asset.clone());
        let index = fetch_pool_bulk_indexes(&self.env, &pool_addr, &request).get_unchecked(0);
        self.market_indexes.set(asset.clone(), index.clone());
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

    pub fn cached_pool_sync_data(&mut self, asset: &Address) -> PoolSyncData {
        if let Some(data) = self.pool_sync_data.get(asset.clone()) {
            return data;
        }
        let pool_addr = self.cached_pool_address();
        let data = fetch_pool_sync_data(&self.env, &pool_addr, asset);
        self.pool_sync_data.set(asset.clone(), data.clone());
        data
    }

    /// Loads the account's e-mode category once per transaction when first needed.
    pub(crate) fn ensure_emode_loaded(&mut self, category_id: u32) {
        if category_id == 0 {
            return;
        }
        if let Some(ctx) = &self.emode_usage {
            assert_with_error!(
                &self.env,
                ctx.category_id() == category_id,
                EModeError::EModeMismatch
            );
            return;
        }
        self.emode_usage = EModeUsageContext::load(&self.env, category_id);
    }

    pub fn cached_emode_asset(
        &mut self,
        category_id: u32,
        asset: &Address,
    ) -> Option<EModeAssetConfig> {
        if category_id == 0 {
            return None;
        }
        self.ensure_emode_loaded(category_id);
        self.emode_usage
            .as_ref()
            .and_then(|ctx| ctx.emode_asset(asset))
    }

    pub fn cached_e_mode_category(&mut self, category_id: u32) -> Option<EModeCategory> {
        if category_id == 0 {
            return None;
        }
        self.ensure_emode_loaded(category_id);
        self.emode_usage.as_ref().map(EModeUsageContext::as_category)
    }

    pub fn active_e_mode_category(&mut self, env: &Env, category_id: u32) -> Option<EModeCategory> {
        let category = self.cached_e_mode_category(category_id)?;
        crate::emode::ensure_e_mode_not_deprecated(env, &Some(category.clone()));
        Some(category)
    }

    pub fn cached_emode_spoke_usage(
        &mut self,
        category_id: u32,
        asset: &Address,
    ) -> Option<EModeSpokeUsageRaw> {
        if category_id == 0 {
            return None;
        }
        self.ensure_emode_loaded(category_id);
        self.emode_usage
            .as_ref()
            .map(|ctx| ctx.spoke_usage(asset))
    }

    pub(crate) fn emode_usage_mut(&mut self, category_id: u32) -> Option<&mut EModeUsageContext> {
        if category_id == 0 {
            return None;
        }
        self.ensure_emode_loaded(category_id);
        self.emode_usage.as_mut()
    }

    pub(crate) fn persist_emode_usage(&self) {
        if let Some(ctx) = &self.emode_usage {
            ctx.persist(&self.env);
        }
    }
}
