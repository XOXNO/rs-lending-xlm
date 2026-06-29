//! Oracle price context methods.

use common::errors::OracleError;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::{
    HubAssetKey, MarketOracleConfig, MarketOracleConfigOption, PriceFeed, PriceFeedRaw,
};
use soroban_sdk::{panic_with_error, Address, String};

use super::Cache;
use crate::oracle::token_price;
use crate::storage;

impl Cache {
    pub fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        (&token_price(self, asset)).into()
    }

    /// USD price for an account position on `spoke_id`. Consults the spoke's
    /// per-asset `oracle_override`: when set, prices `hub_asset` through that
    /// config; otherwise falls back to the token-rooted base
    /// (`cached_price`). The recursive quote-leg resolution inside an override
    /// stays token-rooted (an override reprices the position asset, not the
    /// USD legs its own config quotes against).
    pub fn cached_price_for(&mut self, spoke_id: u32, hub_asset: &HubAssetKey) -> PriceFeed {
        match self.spoke_oracle_override(spoke_id, hub_asset) {
            Some(config) => (&self.spoke_price(hub_asset, &config)).into(),
            None => self.cached_price(&hub_asset.asset),
        }
    }

    /// The override `MarketOracleConfig` the spoke lists for `hub_asset`, or
    /// `None` when the spoke does not list it or leaves it token-rooted.
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

    /// Memoized override price for `hub_asset`, resolved through `config`.
    fn spoke_price(
        &mut self,
        hub_asset: &HubAssetKey,
        config: &MarketOracleConfig,
    ) -> PriceFeedRaw {
        if let Some(feed) = self.spoke_prices.get(hub_asset.clone()) {
            return feed;
        }
        let feed = crate::oracle::price_with_config(self, &hub_asset.asset, config);
        self.spoke_prices.set(hub_asset.clone(), feed.clone());
        feed
    }

    pub fn get_redstone_prefetch(
        &self,
        adapter: &Address,
        feed_id: &String,
    ) -> Option<RedStonePriceData> {
        self.redstone_prefetch
            .get((adapter.clone(), feed_id.clone()))
    }

    pub fn set_redstone_prefetch(
        &mut self,
        adapter: &Address,
        feed_id: &String,
        data: RedStonePriceData,
    ) {
        self.redstone_prefetch
            .set((adapter.clone(), feed_id.clone()), data);
    }

    /// Token-rooted oracle config under `AssetOracle(asset)`, memoized for the
    /// transaction. `None` when the asset has no entry; the `None` case is never
    /// cached, so a disabled asset reverts identically on every touch.
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

    /// Token-rooted oracle config under `AssetOracle(asset)`. Panics
    /// `OracleNotConfigured` when absent. Absence is the pending/disabled gate:
    /// price resolution reverts for any asset with no `AssetOracle` entry.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle_opt(asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }

    /// Whether `asset` has a token-rooted `AssetOracle` entry. Backs the
    /// `require_market_active` pending/disabled gate.
    pub(crate) fn asset_oracle_exists(&mut self, asset: &Address) -> bool {
        self.cached_asset_oracle_opt(asset).is_some()
    }

    /// Token-rooted `AssetOracle` base config for `asset`. This is the bare,
    /// hub-independent price source used by `token_price`, market views, and the
    /// recursive quote legs of any config. The per-spoke `oracle_override` is
    /// resolved one level up in `cached_price_for`, which holds the `(spoke,
    /// hub_asset)` context this path deliberately does not carry.
    pub(crate) fn resolve_oracle_config(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle(asset)
    }
}
