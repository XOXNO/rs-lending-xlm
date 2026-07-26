//! Spoke config memos, listing gates, and per-spoke usage accounting on `Cache`.
//!
//! One spoke bound at a time via `SpokeUsageContext`. Unlisted assets return
//! `None` from `cached_spoke_asset` — callers that treat “paused” as
//! `is_some_and(|c| c.paused)` therefore treat unlisted as not paused.

use common::errors::{GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{AssetConfig, HubAssetKey, MarketIndexRaw, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, panic_with_error, Map};

use crate::context::Cache;
use crate::spoke::{SpokeUsageContext, UsageSide};
use crate::storage;

impl Cache {
    /// Bind `spoke_id` once; a different id later reverts `SpokeMismatch`.
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

    /// Drop spoke usage/config memos so the next account can bind another spoke.
    /// Token-rooted caches (prices, pool sync, market indexes) survive.
    ///
    /// Call only after `persist_spoke_usage` if usage was mutated.
    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
        self.spoke_config = None;
        self.spoke_assets = Map::new(&self.env);
    }

    /// Mutable per-spoke usage context, initializing for `spoke_id` when unset.
    pub(crate) fn require_spoke_usage_context(&mut self, spoke_id: u32) -> &mut SpokeUsageContext {
        self.ensure_spoke_context(spoke_id);
        self.spoke_usage
            .as_mut()
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError))
    }

    /// Per-spoke asset listing for `hub_asset`, or `None` when unlisted.
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

    /// Listed asset config, or panic `AssetNotInSpoke`.
    pub(crate) fn require_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> SpokeAssetConfig {
        self.cached_spoke_asset(spoke_id, hub_asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, SpokeError::AssetNotInSpoke))
    }

    /// Canonical risk-entry gate: the spoke must be active (`SpokeDeprecated`)
    /// and list the asset (`AssetNotInSpoke`); returns the listed risk config.
    pub(crate) fn require_listed_active_config(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> AssetConfig {
        self.active_spoke(spoke_id);
        (&self.require_spoke_asset(spoke_id, hub_asset)).into()
    }

    /// Spoke config from the per-transaction memo (includes deprecated spokes).
    pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        self.ensure_spoke_context(spoke_id);
        if let Some(spoke) = &self.spoke_config {
            return spoke.clone();
        }
        let spoke = storage::get_spoke(&self.env, spoke_id);
        self.spoke_config = Some(spoke.clone());
        spoke
    }

    /// Spoke config, reverting `SpokeDeprecated` when deprecated.
    pub(crate) fn active_spoke(&mut self, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        assert_with_error!(&self.env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
        spoke
    }

    /// Enforces the side's spoke cap, then records the scaled entry delta.
    ///
    /// The listing must exist: entry paths gate on it before reaching the pool,
    /// so a missing row here is an internal inconsistency.
    pub(crate) fn apply_spoke_entry(
        &mut self,
        spoke_id: u32,
        side: UsageSide,
        hub_asset: &HubAssetKey,
        delta_scaled: Ray,
        market_index: &MarketIndexRaw,
        decimals: u32,
    ) {
        let cap = side.cap(
            &self
                .cached_spoke_asset(spoke_id, hub_asset)
                .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError)),
        );
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

    /// Records a scaled exit delta against existing usage.
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

    /// Flush buffered spoke-usage rows to storage (no-op if no spoke bound).
    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
