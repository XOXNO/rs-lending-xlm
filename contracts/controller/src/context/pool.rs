//! Pool address and sync-data context methods.

use common::types::{HubAssetKey, PoolSyncData};
use soroban_sdk::Address;

use super::Cache;
use crate::external::pool::fetch_pool_sync_data;
use crate::storage;

impl Cache {
    /// Address of the central liquidity pool, memoized for the transaction.
    pub fn cached_pool_address(&mut self) -> Address {
        if let Some(addr) = &self.pool_address {
            return addr.clone();
        }
        let addr = storage::get_pool(&self.env);
        self.pool_address = Some(addr.clone());
        addr
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
}
