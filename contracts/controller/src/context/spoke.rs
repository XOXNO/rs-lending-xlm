//! Spoke config and usage context methods.

use common::errors::{GenericError, SpokeError};
use common::types::{HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use super::Cache;
use crate::spoke::SpokeUsageContext;

impl Cache {
    /// Loads the account's spoke once per transaction when first needed. Every
    /// account binds to a real spoke (id `>= 1`), so this always loads a context.
    pub(crate) fn ensure_spoke_loaded(&mut self, spoke_id: u32) {
        if let Some(ctx) = &self.spoke_usage {
            assert_with_error!(
                &self.env,
                ctx.spoke_id() == spoke_id,
                SpokeError::SpokeMismatch
            );
            return;
        }
        self.spoke_usage = SpokeUsageContext::load(&self.env, spoke_id);
    }

    pub fn cached_spoke_asset(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeAssetConfig> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.spoke_usage
            .as_mut()
            .and_then(|ctx| ctx.spoke_asset(&env, hub_asset))
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

    pub fn cached_spoke(&mut self, spoke_id: u32) -> Option<SpokeConfig> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.spoke_usage.as_mut().map(|ctx| ctx.as_spoke(&env))
    }

    pub fn active_spoke(&mut self, env: &Env, spoke_id: u32) -> Option<SpokeConfig> {
        let spoke = self.cached_spoke(spoke_id)?;
        crate::spoke::ensure_spoke_not_deprecated(env, &Some(spoke.clone()));
        Some(spoke)
    }

    pub fn cached_spoke_usage(
        &mut self,
        spoke_id: u32,
        hub_asset: &HubAssetKey,
    ) -> Option<SpokeUsageRaw> {
        self.ensure_spoke_loaded(spoke_id);
        let env = self.env.clone();
        self.spoke_usage
            .as_mut()
            .map(|ctx| ctx.spoke_usage(&env, hub_asset))
    }

    pub(crate) fn spoke_usage_mut(&mut self, spoke_id: u32) -> Option<&mut SpokeUsageContext> {
        self.ensure_spoke_loaded(spoke_id);
        self.spoke_usage.as_mut()
    }

    pub(crate) fn persist_spoke_usage(&self) {
        if let Some(ctx) = &self.spoke_usage {
            ctx.persist(&self.env);
        }
    }
}
