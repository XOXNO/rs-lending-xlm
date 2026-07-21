//! Oracle price and config memos.
//!
//! Position pricing is token-rooted via `cached_price`, cycle-guarded through
//! `enter_price_resolution` / `exit_price_resolution`.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{HubAssetKey, MarketOracleConfig, PriceFeed};
use soroban_sdk::{panic_with_error, Address, String};

use crate::context::Cache;
use crate::oracle::token_price;
use crate::storage;

impl Cache {
    /// Token-rooted USD price for `asset` (cycle-guarded resolution).
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        (&token_price(self, asset)).into()
    }

    /// Position price: token-rooted.
    pub(crate) fn cached_price_for(&mut self, _spoke_id: u32, hub_asset: &HubAssetKey) -> PriceFeed {
        self.cached_price(&hub_asset.asset)
    }

    /// Prefetched RedStone payload for `(adapter, feed_id)`, if any.
    pub(crate) fn get_redstone_prefetch(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.redstone_prefetch
            .get((adapter.clone(), feed_id.clone()))
    }

    /// Store a RedStone payload for the rest of the transaction.
    pub(crate) fn set_redstone_prefetch(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.redstone_prefetch
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Token-rooted oracle config if configured (absence not memoized).
    pub(crate) fn cached_asset_oracle_opt(
        &mut self,
        asset: &Address,
    ) -> Option<MarketOracleConfig> {
        if let Some(config) = self.asset_oracle.get(asset.clone()) {
            return Some(config);
        }
        let config = storage::get_asset_oracle(&self.env, asset)?;
        self.asset_oracle.set(asset.clone(), config.clone());
        Some(config)
    }

    /// Required token-rooted oracle config, or `OracleNotConfigured`.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle_opt(asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }

    /// Whether a token-rooted oracle config exists.
    pub(crate) fn asset_oracle_exists(&mut self, asset: &Address) -> bool {
        self.cached_asset_oracle_opt(asset).is_some()
    }
}
