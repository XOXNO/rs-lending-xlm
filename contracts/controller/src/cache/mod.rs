//! Transaction-local cache for oracle policy, market reads, and batch events.
//!
//! Each mutating entrypoint creates the cache with its `OraclePolicy`; every
//! price and index read then follows that policy for the rest of the call.
//! Position deltas, market snapshots, and isolated-debt changes are buffered
//! until the flow has written storage and emits the final batch events.

use common::constants::MS_PER_SECOND;
use common::events::{
    EventDebtCeilingEntry, EventPositionDelta, UpdateDebtCeilingBatchEvent,
    UpdateMarketStateBatchEvent, UpdatePositionBatchEvent,
};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, DebtPosition, EModeAssetConfig,
    MarketConfig, MarketIndex, MarketIndexRaw, MarketStateSnapshot, PoolSyncData, PriceFeed,
    PriceFeedRaw,
};
use soroban_sdk::{Address, Env, Map, Symbol, Vec};

use crate::cross_contract::pool::fetch_pool_sync_data;
use crate::oracle::policy::OraclePolicy;
use crate::oracle::{token_price, update_asset_index};
use crate::storage;

pub struct Cache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeedRaw>,
    pub market_configs: Map<Address, MarketConfig>,
    pub market_indexes: Map<Address, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<Address, PoolSyncData>,
    emode_assets: Map<(u32, Address), Option<EModeAssetConfig>>,
    isolated_debts: Map<Address, i128>,
    position_updates: Vec<EventPositionDelta>,
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
            market_configs: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            emode_assets: Map::new(env),
            isolated_debts: Map::new(env),
            position_updates: Vec::new(env),
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

    pub fn cached_market_config(&mut self, asset: &Address) -> MarketConfig {
        if let Some(config) = self.market_configs.get(asset.clone()) {
            return config;
        }
        let config = storage::get_market_config(&self.env, asset);
        self.market_configs.set(asset.clone(), config.clone());
        config
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
        let mut updates: Vec<MarketStateSnapshot> = Vec::new(&self.env);
        for mut snapshot in self.market_updates.iter() {
            snapshot.asset_price_wad = self.already_fetched_price(&snapshot.asset);
            updates.push_back(snapshot);
        }
        UpdateMarketStateBatchEvent { updates }.publish(&self.env);
        self.market_updates = Vec::new(&self.env);
    }

    pub fn record_position_update(
        &mut self,
        action: Symbol,
        position_type: AccountPositionType,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.position_updates.push_back(EventPositionDelta::new(
            action,
            position_type,
            asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    pub fn record_debt_position_update(
        &mut self,
        action: Symbol,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.position_updates
            .push_back(EventPositionDelta::new_debt(
                action,
                asset.clone(),
                index_ray,
                amount,
                position,
            ));
    }

    pub fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.position_updates.is_empty() {
            return;
        }
        let mut updates: Vec<EventPositionDelta> = Vec::new(&self.env);
        for mut delta in self.position_updates.iter() {
            delta.asset_price_wad = self.already_fetched_price(&delta.asset);
            updates.push_back(delta);
        }
        UpdatePositionBatchEvent {
            account_id,
            account_attributes: account.into(),
            updates,
        }
        .publish(&self.env);
        self.position_updates = Vec::new(&self.env);
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
            updates.push_back(EventDebtCeilingEntry {
                asset,
                total_debt_usd_wad: value,
            });
        }
        UpdateDebtCeilingBatchEvent { updates }.publish(&self.env);
    }
}
