use super::protocol::{get_shared, set_shared};
use common::errors::{GenericError, SpokeError};
use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{panic_with_error, Env};

/// Reads the last-issued spoke ID from instance storage, increments it by one, stores the new value, and returns it. Panics with `MathOverflow` on overflow.
pub(crate) fn increment_spoke_id(env: &Env) -> u32 {
    let key = ControllerKey::LastSpokeId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

/// Reads a spoke's configuration, panicking with `SpokeError::SpokeNotFound` if it has not been set.
pub(crate) fn get_spoke(env: &Env, id: u32) -> SpokeConfig {
    try_get_spoke(env, id).unwrap_or_else(|| panic_with_error!(env, SpokeError::SpokeNotFound))
}

/// Reads a spoke's configuration from shared persistent storage, or `None` if it has not been set.
pub(crate) fn try_get_spoke(env: &Env, id: u32) -> Option<SpokeConfig> {
    get_shared(env, &ControllerKey::Spoke(id))
}

/// Writes a spoke's configuration to shared persistent storage.
pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    set_shared(env, &ControllerKey::Spoke(id), spoke);
}

/// Reads a spoke's per-asset risk and cap configuration for `hub_asset` from shared persistent storage, or `None` if not configured.
pub(crate) fn get_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    get_shared(env, &ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()))
}

/// Writes a spoke's per-asset risk and cap configuration for `hub_asset` to shared persistent storage.
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

/// Deletes a spoke's per-asset configuration entry for `hub_asset` from persistent storage.
pub(crate) fn remove_spoke_asset(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()));
}

/// Reads a spoke's scaled supplied and borrowed usage (RAY) for `hub_asset` from shared persistent storage, or `None` if not recorded.
pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    get_shared(env, &ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

/// Writes a spoke's scaled supplied/borrowed usage for `hub_asset` to shared persistent storage, removing the entry when both amounts are zero.
pub(crate) fn set_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
    usage: &SpokeUsageRaw,
) {
    let key = ControllerKey::SpokeUsage(spoke_id, hub_asset.clone());

    if usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0 {
        env.storage().persistent().remove(&key);
    } else {
        set_shared(env, &key, usage);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/spoke.rs"]
mod tests;
