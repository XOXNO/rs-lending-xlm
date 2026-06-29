//! Self-contained per-spoke risk resolution.
//!
//! An account on spoke S resolves its risk parameters from
//! `SpokeAsset(S, hub_asset)` directly. Every spoke (id `>= 1`) carries its own
//! full config — there is no spoke 0 base and no base+overlay; each spoke is the
//! single source of truth for its assets.

pub(crate) mod caps;
pub(crate) use caps::SpokeUsageContext;

use common::errors::SpokeError;
use common::types::{AssetConfig, HubAssetKey, SpokeConfig};
use soroban_sdk::{assert_with_error, Env};

use crate::context::Cache;

/// Risk config for the account's spoke, projected to [`AssetConfig`]. Serves
/// `SpokeAsset(spoke_id, hub_asset)` from the per-tx cache memo. A deactivated
/// spoke retains its stored `SpokeAsset` entry, so the read still succeeds; a
/// position on spoke S always reads spoke S. Panics `AssetNotSupported` when
/// the asset is not listed on the spoke.
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
