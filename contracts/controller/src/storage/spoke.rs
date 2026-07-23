//! Spoke config, per-asset config, per-asset usage, and spoke-id allocation.

use crate::storage::renew_protocol_shared_key;
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

pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    let key = ControllerKey::Spoke(id);
    env.storage().persistent().set(&key, spoke);
    renew_protocol_shared_key(env, &key);
}

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
    let key = ControllerKey::SpokeUsage(spoke_id, hub_asset.clone());
    let usage: Option<SpokeUsageRaw> = env.storage().persistent().get(&key);
    // Read-renewal policy matches `get_spoke`/`get_spoke_asset`.
    if usage.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    usage
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
        env.storage().persistent().set(&key, usage);
        renew_protocol_shared_key(env, &key);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/spoke.rs"]
mod tests;
