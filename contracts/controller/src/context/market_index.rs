use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
use soroban_sdk::vec;

#[cfg(not(feature = "certora"))]
use soroban_sdk::Vec;

use crate::context::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    pub(crate) fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    #[cfg(not(feature = "certora"))]
    pub(crate) fn fetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let mut missing: Vec<HubAssetKey> = Vec::new(&self.env);
        for hub_asset in hub_assets.iter() {
            if self.market_indexes.contains_key(hub_asset.clone()) || missing.contains(&hub_asset) {
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
}
