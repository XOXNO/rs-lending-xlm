use common::events::{
    emit_update_debt_ceiling, emit_update_market_state_batch, emit_update_position_batch,
    EventPositionDelta, UpdateDebtCeilingEvent, UpdateMarketStateBatchEvent,
    UpdatePositionBatchEvent,
};
use common::types::{
    Account, AccountPosition, AccountPositionType, AssetConfig, EModeAssetConfig, MarketConfig,
    MarketIndex, MarketStateSnapshot, PoolSyncData, PriceFeed,
};
use soroban_sdk::{Address, Env, Map, Symbol, Vec};

use crate::oracle::policy::OraclePolicy;
use crate::oracle::{token_price, update_asset_index};
use crate::cross_contract::pool::fetch_pool_sync_data;
use crate::storage;

pub struct ControllerCache {
    env: Env,

    pub prices_cache: Map<Address, PriceFeed>,
    pub market_configs: Map<Address, MarketConfig>,
    pub market_indexes: Map<Address, MarketIndex>,
    pool_sync_data: Map<Address, PoolSyncData>,
    emode_assets: Map<(u32, Address), Option<EModeAssetConfig>>,
    isolated_debts: Map<Address, i128>,
    position_updates: Vec<EventPositionDelta>,
    market_updates: Vec<MarketStateSnapshot>,

    pub current_timestamp_ms: u64,
    pub oracle_policy: OraclePolicy,
}

impl ControllerCache {
    pub fn new(env: &Env, oracle_policy: OraclePolicy) -> Self {
        Self::build(env, oracle_policy)
    }

    pub fn new_view(env: &Env) -> Self {
        Self::build(env, OraclePolicy::View)
    }

    pub(crate) fn build(env: &Env, oracle_policy: OraclePolicy) -> Self {
        let current_timestamp_ms = env.ledger().timestamp() * 1000;

        ControllerCache {
            env: env.clone(),
            prices_cache: Map::new(env),
            market_configs: Map::new(env),
            market_indexes: Map::new(env),
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




    pub fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        token_price(self, asset)
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
        self.cached_market_config(asset).asset_config
    }

    pub fn cached_pool_address(&mut self, asset: &Address) -> Address {
        self.cached_market_config(asset).pool_address
    }

    pub fn cached_market_index(&mut self, asset: &Address) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(asset.clone()) {
            return index;
        }
        let index = update_asset_index(self, asset);
        self.market_indexes.set(asset.clone(), index.clone());
        index
    }

    pub fn record_market_update(&mut self, update: &MarketStateSnapshot) {
        self.record_market_update_with_price(update, None);
    }

    pub fn record_market_update_with_price(
        &mut self,
        update: &MarketStateSnapshot,
        asset_price_wad: Option<i128>,
    ) {
        let mut update = update.clone();
        if asset_price_wad.is_some() {
            update.asset_price_wad = asset_price_wad;
        }
        self.market_indexes.set(
            update.asset.clone(),
            MarketIndex {
                borrow_index_ray: update.borrow_index_ray,
                supply_index_ray: update.supply_index_ray,
            },
        );
        self.market_updates.push_back(update);
    }

    pub fn emit_market_batch(&mut self) {
        if self.market_updates.is_empty() {
            return;
        }
        emit_update_market_state_batch(
            &self.env,
            UpdateMarketStateBatchEvent {
                updates: self.market_updates.clone(),
            },
        );
        self.market_updates = Vec::new(&self.env);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_position_update(
        &mut self,
        action: Symbol,
        position_type: AccountPositionType,
        asset: &Address,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
        asset_price_wad: Option<i128>,
    ) {
        self.position_updates.push_back(EventPositionDelta::new(
            action,
            position_type,
            asset.clone(),
            index_ray,
            amount,
            position,
            asset_price_wad,
        ));
    }

    pub fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.position_updates.is_empty() {
            return;
        }
        emit_update_position_batch(
            &self.env,
            UpdatePositionBatchEvent {
                account_id,
                account_attributes: account.into(),
                updates: self.position_updates.clone(),
            },
        );
        self.position_updates = Vec::new(&self.env);
    }

    pub fn cached_pool_sync_data(&mut self, asset: &Address) -> PoolSyncData {
        if let Some(data) = self.pool_sync_data.get(asset.clone()) {
            return data;
        }
        let pool_addr = self.cached_pool_address(asset);
        let data = fetch_pool_sync_data(&self.env, &pool_addr);
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
        for asset in self.isolated_debts.keys() {
            // `keys()` is in lock-step with the underlying map, so `get`
            // cannot return `None`. Surface a typed `InternalError`
            // rather than `.unwrap()` so a future map-invariant break
            // is categorizable instead of an opaque host panic.
            let value = self
                .isolated_debts
                .get(asset.clone())
                .unwrap_or_else(|| {
                    soroban_sdk::panic_with_error!(&self.env, common::errors::GenericError::InternalError)
                });
            storage::set_isolated_debt(&self.env, &asset, value);
            emit_update_debt_ceiling(
                &self.env,
                UpdateDebtCeilingEvent {
                    asset,
                    total_debt_usd_wad: value,
                },
            );
        }
    }
}

