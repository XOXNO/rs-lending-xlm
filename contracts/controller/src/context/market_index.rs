//! Per-invocation cache of pool market indexes on `Cache`, backed by bulk
//! and single-asset fetches from the pool contract.

use common::collections::collect_uncached_keys;
use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
use soroban_sdk::vec;

#[cfg(not(feature = "certora"))]
use soroban_sdk::Vec;

use crate::context::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    /// Stores `index` in the market-index cache for `hub_asset`, overwriting
    /// any previously cached value.
    pub(crate) fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// Fetches and caches market indexes for every entry in `hub_assets`
    /// that is not already cached. Determines the missing keys, then issues
    /// a single bulk fetch against the pool contract for those keys. Does
    /// nothing if all entries are already cached.
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

    /// Returns the market index for `hub_asset`, converted from its cached
    /// raw form. If not cached, fetches it from the pool contract as a
    /// single-entry bulk request, caches the raw result, and returns the
    /// converted value.
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
