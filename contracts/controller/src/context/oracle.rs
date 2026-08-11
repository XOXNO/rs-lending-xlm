//! Per-invocation cache of token price feeds on `Cache`, backed by the
//! price aggregator contract.

use common::collections::collect_uncached_keys;
use common::errors::OracleError;
use common::types::PriceFeed;
#[cfg(test)]
use common::types::PriceFeedRaw;
#[cfg(test)]
use soroban_sdk::Map;
use soroban_sdk::{panic_with_error, Address, Vec};

use crate::context::Cache;

impl Cache {
    /// Replaces the entire cached price map with `prices`. Test-only.
    #[cfg(test)]
    pub(crate) fn set_prices(&mut self, prices: Map<Address, PriceFeedRaw>) {
        self.token_prices = prices;
    }

    /// Fetches and caches price feeds for every entry in `assets` that is
    /// not already cached, via a single call to the price aggregator
    /// contract. Does nothing if all entries are already cached.
    pub(crate) fn fetch_prices(&mut self, assets: &Vec<Address>) {
        let missing = collect_uncached_keys(&self.env, assets, &self.token_prices);
        if missing.is_empty() {
            return;
        }
        let fetched = crate::external::price_aggregator::fetch_prices(&self.env, &missing);
        for (asset, feed) in fetched.iter() {
            self.token_prices.set(asset, feed);
        }
    }

    /// Returns the cached price feed for `asset`, converted from its raw
    /// form. Panics with `OracleError::OracleNotConfigured` if `asset` has
    /// no cached price.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }
}
