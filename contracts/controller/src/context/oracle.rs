//! Oracle price and config memos.
//!
//! Position pricing uses `cached_price_for` (spoke override if listed, else
//! token-rooted). Cycle detection applies on the token path via
//! `enter_price_resolution` / `exit_price_resolution`.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{
    HubAssetKey, MarketOracleConfig, MarketOracleConfigOption, PriceFeed, PriceFeedRaw,
};
use soroban_sdk::{panic_with_error, Address, String};

use crate::context::Cache;
use crate::oracle::{price_with_config, token_price};
use crate::storage;

impl Cache {
    /// Token-rooted USD price for `asset` (cycle-guarded resolution).
    pub(crate) fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        (&token_price(self, asset)).into()
    }

    /// Position price: spoke oracle override if present, else token-rooted.
    pub(crate) fn cached_price_for(&mut self, spoke_id: u32, hub_asset: &HubAssetKey) -> PriceFeed {
        match self.spoke_oracle_override(spoke_id, hub_asset) {
            Some(config) => (&self.spoke_price(hub_asset, &config)).into(),
            None => self.cached_price(&hub_asset.asset),
        }
    }

    /// Override config from the spoke listing, or `None` if unlisted / token-rooted.
    fn spoke_oracle_override(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<MarketOracleConfig> {
        match self
            .cached_spoke_asset(spoke_id, hub_asset)?
            .oracle_override
        {
            MarketOracleConfigOption::Some(config) => Some(config),
            MarketOracleConfigOption::None => None,
        }
    }

    /// Memoized override price for `hub_asset` via `config`.
    fn spoke_price(
        &mut self,
        hub_asset: &HubAssetKey,
        config: &MarketOracleConfig,
    ) -> PriceFeedRaw {
        if let Some(feed) = self.spoke_prices.get(hub_asset.clone()) {
            return feed;
        }
        // Not wrapped in enter/exit: top-level only; nested reads use token
        // oracle (guarded). Guard here too if overrides ever nest into each other.
        let feed = price_with_config(self, &hub_asset.asset, config);
        self.spoke_prices.set(hub_asset.clone(), feed.clone());
        feed
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
