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

    pub(crate) token_prices: Map<Address, PriceFeedRaw>,

    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,

    spoke_usage: Option<SpokeUsageContext>,

    spoke_config: Option<SpokeConfig>,

    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,

    supply_updates: Vec<EventDepositDelta>,

    debt_updates: Vec<EventBorrowDelta>,
}

impl Cache {
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::build(env)
    }

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

    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }
}
