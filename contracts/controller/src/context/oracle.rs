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
    #[cfg(test)]
    pub(crate) fn set_prices(&mut self, prices: Map<Address, PriceFeedRaw>) {
        self.token_prices = prices;
    }

    /// Fetches and caches prices for the entries of `assets` not already cached, via a single call to the price aggregator contract.
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

    /// Returns the price feed cached for `asset`, panicking if none has been fetched into the cache.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }
}

#[cfg(test)]
#[path = "../../tests/context/oracle.rs"]
mod tests;
