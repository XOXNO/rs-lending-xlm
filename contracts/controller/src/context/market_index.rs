use common::collections::collect_uncached_keys;
use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
use soroban_sdk::vec;

#[cfg(not(feature = "certora"))]
use soroban_sdk::Vec;

use crate::context::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    /// Inserts `index` into the market-index cache for `hub_asset`, overwriting any cached value.
    pub(crate) fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// Fetches and caches market indexes for the entries of `hub_assets` not already cached, via a single bulk call to the pool contract.
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

    /// Returns the market index for `hub_asset`, fetching it from the pool contract and caching it if not already cached.
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
