//! Token-rooted USD price lookups from the aggregator-resolved price map.
//!
//! A priced flow calls `ensure_prices` with the asset set it needs; the first
//! call fetches every asset from the price-aggregator in one bulk request, and
//! later calls in the same transaction reuse the cached map (prices are
//! ledger-constant), so a flow with several risk passes still makes a single
//! aggregator call. Per-position reads are map lookups; a missing entry means
//! the asset was never requested — a caller bug — and reverts `OracleNotConfigured`.

use common::errors::OracleError;
#[cfg(test)]
use common::types::PriceFeedRaw;
use common::types::{HubAssetKey, PriceFeed};
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

    /// Resolves any `assets` not yet priced this transaction via one bulk
    /// aggregator call, merging them into the map. Already-cached assets are
    /// skipped, so repeated risk passes share a single fetch.
    pub(crate) fn ensure_prices(&mut self, assets: &Vec<Address>) {
        let env = self.env.clone();
        let mut missing = Vec::new(&env);
        for asset in assets.iter() {
            if !self.token_prices.contains_key(asset.clone()) {
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

    /// Token-rooted USD price for `asset` from the injected map.
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        let raw = self
            .token_prices
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured));
        (&raw).into()
    }

    /// Position price: token-rooted.
    pub(crate) fn cached_price_for(
        &mut self,
        _spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> PriceFeed {
        self.cached_price(&hub_asset.asset)
    }
}
