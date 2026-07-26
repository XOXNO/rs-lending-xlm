//! Spoke config, per-asset config, per-asset usage, and spoke-id allocation.

use crate::storage::{get_shared, set_shared};
use common::errors::{GenericError, SpokeError};
use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{panic_with_error, Env};

/// Allocates and returns the next spoke id, panicking on overflow.
pub(crate) fn increment_spoke_id(env: &Env) -> u32 {
    let key = ControllerKey::LastSpokeId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

pub(crate) fn get_spoke(env: &Env, id: u32) -> SpokeConfig {
    try_get_spoke(env, id).unwrap_or_else(|| panic_with_error!(env, SpokeError::SpokeNotFound))
}

/// Read-renewal policy: stable spokes must not archive while accounts still
/// rely on them.
pub(crate) fn try_get_spoke(env: &Env, id: u32) -> Option<SpokeConfig> {
    get_shared(env, &ControllerKey::Spoke(id))
}

pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    set_shared(env, &ControllerKey::Spoke(id), spoke);
}

pub(crate) fn get_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    get_shared(env, &ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()))
}

pub(crate) fn set_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    config: &SpokeAssetConfig,
) {
    set_shared(
        env,
        &ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()),
        config,
    );
}

pub(crate) fn remove_spoke_asset(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()));
}

pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    // Read-renewal policy matches `get_spoke`/`get_spoke_asset`.
    get_shared(env, &ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

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
        set_shared(env, &key, usage);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/spoke.rs"]
mod tests;
