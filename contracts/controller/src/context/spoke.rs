//! Spoke config and usage context methods.
//!
//! One spoke bound at a time via `SpokeUsageContext`. Unlisted assets return
//! `None` from `cached_spoke_asset` — callers that treat “paused” as
//! `is_some_and(|c| c.paused)` therefore treat unlisted as not paused.

use common::errors::{GenericError, SpokeError};
use common::types::{HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error};

use crate::context::Cache;
use crate::spoke::SpokeUsageContext;

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

    /// Drop spoke usage/config memos and spoke-override prices so the next
    /// account can bind another spoke. Token-rooted caches (prices, oracle
    /// configs, RedStone prefetch, pool sync, market indexes) survive.
    ///
    /// Call only after `persist_spoke_usage` if usage was mutated.
    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
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
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id)
            .spoke_asset(&env, hub_asset)
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

    /// Spoke config from the per-transaction memo (includes deprecated spokes).
    pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id).spoke(&env)
    }

    /// Spoke config, reverting `SpokeDeprecated` when deprecated.
    pub(crate) fn active_spoke(&mut self, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        assert_with_error!(&self.env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
        spoke
    }

    /// Buffered per-spoke usage for `hub_asset`, lazily loaded from storage.
    pub(crate) fn cached_spoke_usage(&mut self, spoke_id: u32, hub_asset: &HubAssetKey) -> SpokeUsageRaw {
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id)
            .spoke_usage(&env, hub_asset)
    }

    /// Flush buffered spoke-usage rows to storage (no-op if no spoke bound).
    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
