//! Self-contained per-spoke risk resolution.
//!
//! An account on spoke S resolves its risk parameters from
//! `SpokeAsset(S, hub_asset)` directly. Every spoke (id `>= 1`) carries its own
//! full config — there is no spoke 0 base and no base+overlay; each spoke is the
//! single source of truth for its assets.

use common::errors::{SpokeError, GenericError};
use controller_interface::types::{AssetConfig, HubAssetKey, SpokeAssetConfig, SpokeConfig};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::cache::Cache;
use crate::storage;

/// Per-spoke risk config for `hub_asset` on `spoke_id`. Each spoke holds its own
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
/// `SpokeAsset(spoke_id, hub_asset)` directly. A deactivated spoke retains its
/// stored `SpokeAsset` entry, so the read still succeeds; a position on spoke S
/// always reads spoke S.
pub fn effective_asset_config(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) -> AssetConfig {
    (&resolve_spoke_asset_config(env, spoke_id, hub_asset)).into()
}

pub fn ensure_spoke_not_deprecated(env: &Env, spoke: &Option<SpokeConfig>) {
    if let Some(spoke) = spoke {
        assert_with_error!(
            env,
            !spoke.is_deprecated,
            SpokeError::SpokeDeprecated
        );
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
#[path = "../tests/spoke.rs"]
mod tests;
