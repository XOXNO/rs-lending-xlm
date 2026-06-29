//! Pool market-index context methods.

use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
#[cfg(not(feature = "certora"))]
use soroban_sdk::Map;
use soroban_sdk::Vec;

use super::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    /// Caches an index the pool returned from a mutation (`PoolPositionMutation.
    /// market_index`). Lets the post-action valuation skip a redundant pool read
    /// for the touched hub-asset.
    pub fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// Certora stub: lazy per-asset reads preserve semantics.
    #[cfg(feature = "certora")]
    pub fn prefetch_market_indexes(&mut self, _hub_assets: &Vec<HubAssetKey>) {}

    /// Seeds `market_indexes` for uncached hub-assets.
    /// Skips duplicates and markets already loaded in this transaction. The pool
    /// reverts `PoolNotInitialized` for any uncreated (hub, asset) in the batch.
    #[cfg(not(feature = "certora"))]
    pub fn prefetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let mut missing: Vec<HubAssetKey> = Vec::new(&self.env);
        let mut seen: Map<HubAssetKey, bool> = Map::new(&self.env);
        for hub_asset in hub_assets.iter() {
            if self.market_indexes.contains_key(hub_asset.clone())
                || seen.contains_key(hub_asset.clone())
            {
                continue;
            }
            seen.set(hub_asset.clone(), true);
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

    /// Returns the pool-sourced index for `hub_asset`. On a cache miss the pool is
    /// asked for it (single-asset `bulk_get_indexes`); the controller never
    /// simulates accrual itself.
    pub fn cached_market_index(&mut self, hub_asset: &HubAssetKey) -> MarketIndex {
        if let Some(index) = self.market_indexes.get(hub_asset.clone()) {
            return (&index).into();
        }
        let pool_addr = self.cached_pool_address();
        let mut request = Vec::new(&self.env);
        request.push_back(hub_asset.clone());
        let index = fetch_pool_bulk_indexes(&self.env, &pool_addr, &request).get_unchecked(0);
        self.market_indexes.set(hub_asset.clone(), index.clone());
        (&index).into()
    }
}
