use common::types::{HubAssetKey, PoolSyncData};
use soroban_sdk::Address;

use crate::context::Cache;
use crate::external::pool::fetch_pool_sync_data;
use crate::storage;

impl Cache {
    /// Returns the pool contract's address, reading and caching it from storage on first call.
    pub(crate) fn cached_pool_address(&mut self) -> Address {
        if let Some(addr) = &self.pool_address {
            return addr.clone();
        }
        let addr = storage::get_pool(&self.env);
        self.pool_address = Some(addr.clone());
        addr
    }

    /// Returns the pool's sync data for `hub_asset`, fetching it from the pool contract and caching it if not already cached.
    pub(crate) fn cached_pool_sync_data(&mut self, hub_asset: &HubAssetKey) -> PoolSyncData {
        if let Some(data) = self.pool_sync_data.get(hub_asset.clone()) {
            return data;
        }
        let pool_addr = self.cached_pool_address();
        let data = fetch_pool_sync_data(&self.env, &pool_addr, hub_asset);
        self.pool_sync_data.set(hub_asset.clone(), data.clone());
        data
    }
}
