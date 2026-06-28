//! Self-contained per-spoke risk resolution.
//!
//! An account on spoke S resolves its risk parameters from
//! `SpokeAsset(S, hub_asset)` directly; the general spoke 0 holds every listed
//! asset's base config. There is no base+overlay — each spoke is self-contained.

use common::errors::{EModeError, GenericError};
use controller_interface::types::{AssetConfig, HubAssetKey, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;
use crate::storage;

/// Per-spoke risk config for `hub_asset` on `spoke_id`. The general spoke 0 is
/// every listed asset's base listing; named spokes hold their own
/// self-contained config. Panics `AssetNotSupported` when the asset is not
/// listed on the spoke.
pub fn resolve_spoke_asset_config(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> SpokeAssetConfig {
    storage::get_spoke_asset(env, spoke_id, hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AssetNotSupported))
}

/// Risk config for the account's spoke, projected to [`AssetConfig`]. Reads
/// `SpokeAsset(spoke_id, hub_asset)` directly with no overlay onto spoke 0. A
/// deactivated spoke retains its stored `SpokeAsset` entry, so the read still
/// succeeds; a position on spoke S always reads spoke S (no spoke-0 fallback).
pub fn effective_asset_config(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) -> AssetConfig {
    (&resolve_spoke_asset_config(env, spoke_id, hub_asset)).into()
}

pub fn ensure_spoke_not_deprecated(env: &Env, spoke: &Option<SpokeConfig>) {
    if let Some(spoke) = spoke {
        assert_with_error!(
            env,
            !spoke.is_deprecated,
            EModeError::EModeCategoryDeprecated
        );
    }
}

/// Asserts the asset is listed on `spoke_id` and the owning spoke is active.
/// Spoke 0 always lists every created asset and is never deprecated, so this is
/// a no-op there.
pub fn validate_spoke_lists_asset(
    env: &Env,
    cache: &mut Cache,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) {
    if spoke_id == 0 {
        return;
    }
    assert_with_error!(
        env,
        cache.cached_spoke_asset(spoke_id, hub_asset).is_some(),
        EModeError::EModeCategoryNotFound
    );
    // Rejects a deprecated spoke (EModeCategoryDeprecated).
    cache.active_spoke(env, spoke_id);
}

#[cfg(test)]
#[path = "../tests/emode.rs"]
mod tests;
