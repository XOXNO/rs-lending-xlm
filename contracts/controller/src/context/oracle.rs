//! Oracle price context methods.

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
    pub fn cached_price(&mut self, asset: &Address) -> PriceFeed {
        (&token_price(self, asset)).into()
    }

    /// Resolves position price using spoke override, else token-rooted oracle.
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
        // Deliberately not wrapped in enter/exit_price_resolution: this override
        // entry is only reached from top-level, non-recursive callers, and internal
        // resolution (compose/reflector) always reads the base `AssetOracle`, never
        // this override — so it can't form a cycle back to itself. Any chaining it
        // triggers still routes through the guarded `token_price`. If overrides are
        // ever made to participate in nested resolution, guard this call too.
        let feed = price_with_config(self, &hub_asset.asset, config);
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

    /// Token-rooted oracle config; missing entries are not cached.
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

    /// Required token-rooted oracle config.
    pub(crate) fn cached_asset_oracle(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle_opt(asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, OracleError::OracleNotConfigured))
    }

    /// Token-rooted oracle presence gate.
    pub(crate) fn asset_oracle_exists(&mut self, asset: &Address) -> bool {
        self.cached_asset_oracle_opt(asset).is_some()
    }

    /// Token-rooted oracle config for bare asset; no spoke context here.
    pub(crate) fn resolve_oracle_config(&mut self, asset: &Address) -> MarketOracleConfig {
        self.cached_asset_oracle(asset)
    }
}
