//! Spoke config, per-asset config, and per-asset usage storage.

use crate::storage::renew_protocol_shared_key;
use common::errors::SpokeError;
use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{panic_with_error, Env};

/// Returns the spoke config, panicking if the spoke is not found.
pub(crate) fn get_spoke(env: &Env, id: u32) -> SpokeConfig {
    try_get_spoke(env, id).unwrap_or_else(|| panic_with_error!(env, SpokeError::SpokeNotFound))
}

/// Returns the spoke config if it exists.
pub(crate) fn try_get_spoke(env: &Env, id: u32) -> Option<SpokeConfig> {
    let key = ControllerKey::Spoke(id);
    let spoke: Option<SpokeConfig> = env.storage().persistent().get(&key);
    // Read-renewal policy: stable spokes must not archive while accounts still
    // rely on them.
    if spoke.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    spoke
}

/// Stores a spoke config and renews its shared-tier TTL.
pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    let key = ControllerKey::Spoke(id);
    env.storage().persistent().set(&key, spoke);
    renew_protocol_shared_key(env, &key);
}

/// Returns the per-spoke asset config if present.
pub(crate) fn get_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    let key = ControllerKey::SpokeAsset(spoke_id, hub_asset.clone());
    let config: Option<SpokeAssetConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    config
}

/// Stores a per-spoke asset config and renews its shared-tier TTL.
pub(crate) fn set_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    config: &SpokeAssetConfig,
) {
    let key = ControllerKey::SpokeAsset(spoke_id, hub_asset.clone());
    env.storage().persistent().set(&key, config);
    renew_protocol_shared_key(env, &key);
}

/// Removes the per-spoke asset config entry.
pub(crate) fn remove_spoke_asset(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()));
}

/// Returns the per-spoke asset usage if present.
pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    env.storage()
        .persistent()
        .get(&ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

/// Stores per-spoke asset usage, pruning the entry when fully zero.
pub(crate) fn set_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    usage: &SpokeUsageRaw,
) {
    let key = ControllerKey::SpokeUsage(spoke_id, hub_asset.clone());
    // A fully-zero entry carries no information; prune it so empty usage does
    // not occupy storage.
    if usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, usage);
        renew_protocol_shared_key(env, &key);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/spoke.rs"]
mod tests;
