//! Pool market-index memos.
//!
//! Indexes are pool truth only — never simulated on the controller. After a
//! mutation, `put_market_index` stores the returned index so post-action risk
//! skips a redundant pool read. Bulk prefetch assumes results align with the
//! request order.

use common::types::{HubAssetKey, MarketIndex, MarketIndexRaw};
use soroban_sdk::{vec, Vec};

use crate::context::Cache;
use crate::external::pool::fetch_pool_bulk_indexes;

impl Cache {
    /// Store the index returned on a pool mutation for this hub-asset.
    pub(crate) fn put_market_index(&mut self, hub_asset: &HubAssetKey, index: &MarketIndexRaw) {
        self.market_indexes.set(hub_asset.clone(), index.clone());
    }

    /// No-op under Certora; the harness supplies indexes directly.
    #[cfg(feature = "certora")]
    pub(crate) fn prefetch_market_indexes(&mut self, _hub_assets: &Vec<HubAssetKey>) {}

    /// Bulk-load indexes for uncached hub-assets (deduped). Pool reverts
    /// `PoolNotInitialized` for any uncreated market in the batch.
    #[cfg(not(feature = "certora"))]
    pub(crate) fn prefetch_market_indexes(&mut self, hub_assets: &Vec<HubAssetKey>) {
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

    /// Pool-sourced index for `hub_asset` (fetch and memoize on miss).
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
