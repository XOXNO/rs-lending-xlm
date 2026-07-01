//! Per-spoke risk resolution from `SpokeAsset(spoke_id, hub_asset)`.

pub(crate) mod caps;
pub(crate) use caps::SpokeUsageContext;

use common::errors::SpokeError;
use common::types::{AssetConfig, HubAssetKey, SpokeConfig};
use soroban_sdk::{assert_with_error, Env};

use crate::context::Cache;

/// Risk config for the account's spoke.
pub fn effective_asset_config(
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> AssetConfig {
    (&cache.require_spoke_asset(spoke_id, hub_asset)).into()
}

pub fn ensure_spoke_not_deprecated(env: &Env, spoke: &Option<SpokeConfig>) {
    if let Some(spoke) = spoke {
        assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    }
}

/// Asserts the asset is listed on `spoke_id` and the owning spoke is active.
pub fn validate_spoke_lists_asset(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) {
    assert_with_error!(
        env,
        cache.cached_spoke_asset(spoke_id, hub_asset).is_some(),
        SpokeError::SpokeNotFound
    );
    // Rejects a deprecated spoke (SpokeDeprecated).
    cache.active_spoke(env, spoke_id);
}

#[cfg(test)]
#[path = "../../tests/spoke.rs"]
mod tests;
