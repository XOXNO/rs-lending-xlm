//! Spoke config and usage context methods.

use common::errors::{GenericError, SpokeError};
use common::types::{HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use super::Cache;
use crate::spoke::SpokeUsageContext;

impl Cache {
    /// Initializes account spoke context once per transaction. Every account
    /// binds one real spoke (id `>= 1`); mixing spokes in one cache is invalid.
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

    /// Drops the per-spoke context (usage buffer, spoke config, spoke-asset
    /// memo) and the spoke-scoped override price cache so the next account can
    /// bind a different spoke. Token-rooted caches (prices, oracle configs,
    /// RedStone prefetch, pool sync data, market indexes) are spoke-independent
    /// and survive, preserving the cross-contract savings of a shared batch
    /// cache. Only valid between accounts, after any pending usage writes were
    /// persisted (or when the flow never mutates usage).
    pub(crate) fn reset_spoke_context(&mut self) {
        self.spoke_usage = None;
        self.spoke_prices = soroban_sdk::Map::new(&self.env);
    }

    pub(crate) fn require_spoke_usage_context(&mut self, spoke_id: u32) -> &mut SpokeUsageContext {
        self.ensure_spoke_context(spoke_id);
        self.spoke_usage
            .as_mut()
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::InternalError))
    }

    pub fn cached_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id)
            .spoke_asset(&env, hub_asset)
    }

    /// Per-spoke risk config for `hub_asset` on `spoke_id`, served from the
    /// per-tx memo. Panics `AssetNotSupported` when the asset is not listed on
    /// the spoke (the absence revert that risk resolution depends on).
    pub(crate) fn require_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> SpokeAssetConfig {
        self.cached_spoke_asset(spoke_id, hub_asset)
            .unwrap_or_else(|| panic_with_error!(&self.env, GenericError::AssetNotSupported))
    }

    pub fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
        let env = self.env.clone();
        self.require_spoke_usage_context(spoke_id).spoke(&env)
    }

    pub fn active_spoke(&mut self, env: &Env, spoke_id: u32) -> SpokeConfig {
        let spoke = self.spoke_config(spoke_id);
        crate::spoke::ensure_spoke_not_deprecated(env, &Some(spoke.clone()));
        spoke
    }

    pub fn cached_spoke_usage(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeUsageRaw> {
        let env = self.env.clone();
        Some(
            self.require_spoke_usage_context(spoke_id)
                .spoke_usage(&env, hub_asset),
        )
    }

    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
