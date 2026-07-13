//! Pool market-index context methods.

use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
use soroban_sdk::{vec, Vec};

use crate::context::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    /// Caches an index the pool returned from a mutation (`PoolPositionMutation.
    /// market_index`). Lets the post-action valuation skip a redundant pool read
    /// for the touched hub-asset.
    pub fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// No-op under Certora; the harness supplies market indexes directly.
    #[cfg(feature = "certora")]
    pub fn prefetch_market_indexes(&mut self, _hub_assets: &Vec<HubAssetKey>) {}

    /// Seeds `market_indexes` for uncached hub-assets.
    /// Skips duplicates and markets already loaded in this transaction. The pool
    /// reverts `PoolNotInitialized` for any uncreated (hub, asset) in the batch.
    #[cfg(not(feature = "certora"))]
    pub fn prefetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
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

    /// Returns the pool-sourced index for `hub_asset`. On a cache miss the pool is
    /// asked for it (single-asset `get_bulk_indexes`); the controller never
    /// simulates accrual itself.
    pub fn cached_market_index(&mut self, hub_asset: &HubAssetKey) -> MarketIndex {
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
