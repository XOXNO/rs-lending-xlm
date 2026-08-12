
use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{AssetConfig, HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, panic_with_error, Map};

use crate::context::Cache;
use crate::spoke::{SpokeUsageContext, UsageSide};
use crate::storage;

impl Cache {
    pub(crate) fn ensure_spoke_context(&mut self, spoke_id: u32) {
        if let Some(ctx) = &self.spoke_usage {
            assert_with_error!(
                &self.env,
                ctx.spoke_id() == spoke_id,
                SpokeError::SpokeMismatch
            );
            return;
        }
        self.spoke_usage = Some(SpokeUsageContext::new(&self.env, spoke_id));
    }

    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
        self.spoke_config = None;
        self.spoke_assets = Map::new(&self.env);
    }

    pub(crate) fn require_spoke_usage_context(&mut self, spoke_id: u32) -> &mut SpokeUsageContext {
        self.ensure_spoke_context(spoke_id);
        self.spoke_usage
            .as_mut()
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError))
    }

    pub(crate) fn cached_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        self.ensure_spoke_context(spoke_id);
        if let Some(cfg) = self.spoke_assets.get(hub_asset.clone()) {
            return Some(cfg);
        }
        let loaded = storage::get_spoke_asset(&self.env, spoke_id, hub_asset)?;
        self.spoke_assets.set(hub_asset.clone(), loaded.clone());
        Some(loaded)
    }

    pub(crate) fn require_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        let asset = self.require_spoke_asset_config(spoke_id, hub_asset);

        (&asset).into()
    }

    pub(crate) fn require_spoke_asset_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> SpokeAssetConfig {
        self.cached_spoke_asset(spoke_id, hub_asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, SpokeError::AssetNotInSpoke))
    }

    pub(crate) fn require_listed_active_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        self.active_spoke(spoke_id);
        self.require_spoke_asset(spoke_id, hub_asset)
    }

    pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        self.ensure_spoke_context(spoke_id);
        if let Some(spoke) = &self.spoke_config {
            return spoke.clone();
        }
        let spoke = storage::get_spoke(&self.env, spoke_id);
        self.spoke_config = Some(spoke.clone());
        spoke
    }

    pub(crate) fn active_spoke(&mut self, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        assert_with_error!(&self.env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
        spoke
    }

    pub(crate) fn apply_spoke_entry(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let spoke_config = self.require_spoke_asset_config(spoke_id, hub_asset);
        let cap = side.cap(&spoke_config);
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id).apply_entry(
            &env,
            side,
            hub_asset,
            delta_scaled,
            cap,
            side.index(market_index),
            decimals,
        );
    }

    pub(crate) fn apply_spoke_exit(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
    ) {
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id)
            .apply_exit(&env, side, hub_asset, delta_scaled);
    }

    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
