//! Token-rooted USD price lookups from the aggregator-resolved price map.
//!
//! A priced flow calls [`Cache::fetch_prices`] (or [`Cache::load_markets`] when
//! hub-asset indexes are needed too). The first call bulk-fetches uncached
//! assets from the price-aggregator; later calls in the same transaction reuse
//! the map (prices are ledger-constant). Per-position reads are map lookups; a
//! missing entry means the asset was never fetched — a caller bug — and reverts
//! `OracleNotConfigured`.

use common::errors::OracleError;
use common::types::PriceFeed;
#[cfg(test)]
use common::types::PriceFeedRaw;
#[cfg(test)]
use soroban_sdk::Map;
use soroban_sdk::{panic_with_error, Address, Vec};

use crate::context::Cache;

impl Cache {
    /// Injects a price map directly (test helper).
    #[cfg(test)]
    pub(crate) fn set_prices(&mut self, prices: Map<Address, PriceFeedRaw>) {
        self.token_prices = prices;
    }

    /// Bulk-fetch USD prices for any `assets` not yet in this transaction's map.
    ///
    /// Already-cached tokens are skipped, so repeated risk passes share one
    /// aggregator call. Prefer [`Cache::load_markets`] when you also need pool
    /// indexes for the same hub-asset set.
    pub(crate) fn fetch_prices(&mut self, assets: &Vec<Address>) {
        let env = self.env.clone();
        let mut missing = Vec::new(&env);
        for asset in assets.iter() {
            if !self.token_prices.contains_key(asset.clone()) && !missing.contains(&asset) {
                missing.push_back(asset);
            }
        }
        if missing.is_empty() {
            return;
        }
        let fetched = crate::external::price_aggregator::fetch_prices(&env, &missing);
        for (asset, feed) in fetched.iter() {
            self.token_prices.set(asset, feed);
        }
    }

    /// Token-rooted USD price for `asset` from the fetched map.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }
}
