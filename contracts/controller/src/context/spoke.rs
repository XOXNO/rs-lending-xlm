//! Per-invocation memoization of spoke config, spoke asset config, and
//! spoke usage state on `Cache`.

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{AssetConfig, HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, panic_with_error, Map};

use crate::context::Cache;
use crate::spoke::{SpokeUsageContext, UsageSide};
use crate::storage;

impl Cache {
    /// Ensures a [`SpokeUsageContext`] for `spoke_id` exists on the cache,
    /// creating one if absent. Panics with `SpokeError::SpokeMismatch` if a
    /// context for a different spoke is already cached, since a single
    /// `Cache` only tracks usage for one spoke per invocation.
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

    /// Clears the cached spoke usage context, spoke config, and spoke asset
    /// configs, so the next access reloads them from storage.
    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
        self.spoke_config = None;
        self.spoke_assets = Map::new(&self.env);
    }

    /// Ensures the spoke usage context for `spoke_id` is cached, then
    /// returns a mutable reference to it. Panics with
    /// `GenericError::InternalError` if the context is unexpectedly absent
    /// after `ensure_spoke_context` runs.
    pub(crate) fn require_spoke_usage_context(&mut self, spoke_id: u32) -> &mut SpokeUsageContext {
        self.ensure_spoke_context(spoke_id);
        self.spoke_usage
            .as_mut()
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError))
    }

    /// Returns the cached `SpokeAssetConfig` for `hub_asset` in `spoke_id`,
    /// loading it from storage and caching it on a miss. Returns `None` if
    /// the asset is not listed in the spoke.
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

    /// Returns the `AssetConfig` for `hub_asset` in `spoke_id`, converted
    /// from the spoke's `SpokeAssetConfig`. Panics with
    /// `SpokeError::AssetNotInSpoke` if the asset is not listed in the
    /// spoke.
    pub(crate) fn require_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        let asset = self.require_spoke_asset_config(spoke_id, hub_asset);

        (&asset).into()
    }

    /// Returns the `SpokeAssetConfig` for `hub_asset` in `spoke_id`. Panics
    /// with `SpokeError::AssetNotInSpoke` if the asset is not listed in the
    /// spoke.
    pub(crate) fn require_spoke_asset_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> SpokeAssetConfig {
        self.cached_spoke_asset(spoke_id, hub_asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, SpokeError::AssetNotInSpoke))
    }

    /// Returns the `AssetConfig` for `hub_asset` in `spoke_id`, first
    /// asserting that `spoke_id` is active. Panics with
    /// `SpokeError::SpokeDeprecated` if the spoke is deprecated, or
    /// `SpokeError::AssetNotInSpoke` if the asset is not listed in the
    /// spoke.
    pub(crate) fn require_listed_active_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        self.active_spoke(spoke_id);
        self.require_spoke_asset(spoke_id, hub_asset)
    }

    /// Returns the cached `SpokeConfig` for `spoke_id`, loading it from
    /// storage and caching it on a miss.
    pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        self.ensure_spoke_context(spoke_id);
        if let Some(spoke) = &self.spoke_config {
            return spoke.clone();
        }
        let spoke = storage::get_spoke(&self.env, spoke_id);
        self.spoke_config = Some(spoke.clone());
        spoke
    }

    /// Returns the `SpokeConfig` for `spoke_id` after asserting it is not
    /// deprecated. Panics with `SpokeError::SpokeDeprecated` if the spoke is
    /// deprecated.
    pub(crate) fn active_spoke(&mut self, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        assert_with_error!(&self.env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
        spoke
    }

    /// Increases the cached spoke usage for `hub_asset` on `side` by
    /// `delta_scaled`, enforcing the spoke's supply or borrow cap (per
    /// `side`) against the corresponding market index and asset decimals.
    /// Panics with the side's cap error if the resulting usage exceeds the
    /// cap, or with `GenericError::MathOverflow` on overflow.
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

    /// Decreases the cached spoke usage for `hub_asset` on `side` by
    /// `delta_scaled`. Is a no-op if `delta_scaled` is zero or if no usage
    /// row exists yet for `hub_asset`. Panics with
    /// `GenericError::MathOverflow` on subtraction overflow, or
    /// `GenericError::InternalError` if the result would go negative.
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

    /// Writes all cached spoke usage rows to persistent storage. Is a no-op
    /// if no spoke usage context is cached.
    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
