//! Transaction-local cache for oracle policy, market reads, and batch events.
//!
//! Each mutating entrypoint creates the cache with its `OraclePolicy`; every
//! price and index read then follows that policy for the rest of the call.
//! Position deltas, market snapshots, and isolated-debt changes are buffered
//! until the flow has written storage and emits the final batch events.

use common::constants::MS_PER_SECOND;
use common::events::{
    EventBorrowDelta, EventDebtCeilingEntry, EventDepositDelta, EventMarketState, PositionAction,
    UpdateDebtCeilingBatchEvent, UpdateMarketStateBatchEvent, UpdatePositionBatchEvent,
};
use common::types::{
    Account, AccountPosition, AssetConfig, DebtPosition, EModeAssetConfig,
    MarketConfig, MarketIndex, MarketIndexRaw, MarketStateSnapshot, PoolSyncData, PriceFeed,
    PriceFeedRaw,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, String, Vec};

#[cfg(not(feature = "certora"))]
use crate::cross_contract::pool::fetch_pool_bulk_indexes;
use crate::cross_contract::pool::fetch_pool_sync_data;
use crate::oracle::policy::OraclePolicy;
use crate::oracle::providers::redstone::RedStonePriceData;
use crate::oracle::{token_price, update_asset_index};
use crate::storage;

pub struct Cache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeedRaw>,
    /// Raw RedStone payloads bulk-fetched once per tx, keyed by (adapter, feed_id).
    /// Stores provider data, never resolved prices, so per-flow policy checks
    /// (staleness, sanity, tolerance) are unaffected.
    redstone_prefetch: Map<(Address, String), RedStonePriceData>,
    pub market_configs: Map<Address, MarketConfig>,
    pub market_indexes: Map<Address, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<Address, PoolSyncData>,
    emode_assets: Map<(u32, Address), Option<EModeAssetConfig>>,
    isolated_debts: Map<Address, i128>,
    deposit_updates: Vec<EventDepositDelta>,
    borrow_updates: Vec<EventBorrowDelta>,
    market_updates: Vec<MarketStateSnapshot>,

    pub current_timestamp_ms: u64,
    pub oracle_policy: OraclePolicy,
}

impl Cache {
    /// Creates a cache for mutating flows and renews controller instance TTL.
    pub fn new(env: &Env, oracle_policy: OraclePolicy) -> Self {
        crate::storage::renew_controller_instance(env);
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
            emode_assets: Map::new(env),
            isolated_debts: Map::new(env),
            deposit_updates: Vec::new(env),
            borrow_updates: Vec::new(env),
            market_updates: Vec::new(env),
            current_timestamp_ms,
            oracle_policy,
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

    /// No-op under Certora: pure performance optimization, identical semantics.
    #[cfg(feature = "certora")]
    pub fn prefetch_market_indexes(&mut self, _assets: &Vec<Address>) {}

    /// Seeds `market_indexes` for every listed asset in `assets` with one
    /// `bulk_get_sync_data` pool call instead of N lazy `get_sync_data` reads.
    ///
    /// The pool runs the same `simulate_update_indexes` math the lazy
    /// per-asset path runs locally, so seeded values are identical. Assets
    /// already indexed this tx and unlisted assets are skipped — the prefetch
    /// never introduces its own panic site — and an empty remainder makes no
    /// pool call, so flows that never read an index stay call-free.
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
        // Index-aligned with the request: the pool returns one entry per asset.
        for (i, asset) in missing.iter().enumerate() {
            self.market_indexes
                .set(asset, indexes.get_unchecked(i as u32));
        }
    }

    pub fn cached_market_index(&mut self, asset: &Address) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(asset.clone()) {
            return (&index).into();
        }
        let index = update_asset_index(self, asset);
        self.market_indexes
            .set(asset.clone(), MarketIndexRaw::from(&index));
        index
    }

    pub fn record_market_update(&mut self, update: &MarketStateSnapshot) {
        self.market_indexes.set(
            update.asset.clone(),
            MarketIndexRaw {
                borrow_index_ray: update.borrow_index_ray,
                supply_index_ray: update.supply_index_ray,
            },
        );
        self.market_updates.push_back(update.clone());
    }

    /// Price already fetched this transaction, if any. Event enrichment reads
    /// only this memo and never triggers an oracle call of its own, so flows
    /// whose risk checks need no price (e.g. a debt-free full exit) stay
    /// oracle-free end to end.
    fn already_fetched_price(&self, asset: &Address) -> Option<i128> {
        self.prices_cache.get(asset.clone()).map(|f| f.price_wad)
    }

    pub fn emit_market_batch(&mut self) {
        if self.market_updates.is_empty() {
            return;
        }
        let mut updates: Vec<EventMarketState> = Vec::new(&self.env);
        for mut snapshot in self.market_updates.iter() {
            snapshot.asset_price_wad = self.already_fetched_price(&snapshot.asset);
            updates.push_back(EventMarketState::from(&snapshot));
        }
        UpdateMarketStateBatchEvent { updates }.publish(&self.env);
        self.market_updates = Vec::new(&self.env);
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

    pub fn cached_emode_asset(
        &mut self,
        category_id: u32,
        asset: &Address,
    ) -> Option<EModeAssetConfig> {
        if category_id == 0 {
            return None;
        }
        let key = (category_id, asset.clone());
        if let Some(cached) = self.emode_assets.get(key.clone()) {
            return cached;
        }
        let value = storage::get_emode_asset(&self.env, category_id, asset);
        self.emode_assets.set(key, value.clone());
        value
    }

    pub fn get_isolated_debt(&mut self, asset: &Address) -> i128 {
        if let Some(v) = self.isolated_debts.get(asset.clone()) {
            return v;
        }
        let v = storage::get_isolated_debt(&self.env, asset);
        self.isolated_debts.set(asset.clone(), v);
        v
    }

    pub fn set_isolated_debt(&mut self, asset: &Address, value: i128) {
        self.isolated_debts.set(asset.clone(), value);
    }

    pub fn flush_isolated_debts(&self) {
        if self.isolated_debts.is_empty() {
            return;
        }
        let mut updates: Vec<EventDebtCeilingEntry> = Vec::new(&self.env);
        for (asset, value) in self.isolated_debts.iter() {
            storage::set_isolated_debt(&self.env, &asset, value);
            updates.push_back(EventDebtCeilingEntry(asset, value));
        }
        UpdateDebtCeilingBatchEvent { updates }.publish(&self.env);
    }
}
