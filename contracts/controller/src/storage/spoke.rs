//! Storage accessors for spoke configuration, spoke ID allocation, per-spoke asset
//! configuration, and per-spoke asset usage (supplied/borrowed totals).

use crate::storage::{get_shared, set_shared};
use common::errors::{GenericError, SpokeError};
use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{panic_with_error, Env};

/// Allocates and returns the next spoke ID, starting from 1. Persists the updated counter
/// in instance storage. Panics with `MathOverflow` if the counter would overflow `u32`.
pub(crate) fn increment_spoke_id(env: &Env) -> u32 {
    let key = ControllerKey::LastSpokeId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

/// Reads the configuration for spoke `id`. Panics with `SpokeNotFound` if it does not exist.
pub(crate) fn get_spoke(env: &Env, id: u32) -> SpokeConfig {
    try_get_spoke(env, id).unwrap_or_else(|| panic_with_error!(env, SpokeError::SpokeNotFound))
}

/// Reads the configuration for spoke `id`. Returns `None` if it does not exist.
pub(crate) fn try_get_spoke(env: &Env, id: u32) -> Option<SpokeConfig> {
    get_shared(env, &ControllerKey::Spoke(id))
}

/// Writes the configuration for spoke `id`.
pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    set_shared(env, &ControllerKey::Spoke(id), spoke);
}

/// Reads the configuration of `hub_asset` on spoke `spoke_id`. Returns `None` if it is not
/// listed on that spoke.
pub(crate) fn get_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    get_shared(env, &ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()))
}

/// Writes the configuration of `hub_asset` on spoke `spoke_id`.
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

/// Removes the configuration of `hub_asset` on spoke `spoke_id`.
pub(crate) fn remove_spoke_asset(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()));
}

/// Reads the usage (supplied/borrowed totals) of `hub_asset` on spoke `spoke_id`. Returns
/// `None` if no usage is stored.
pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    get_shared(env, &ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

/// Writes the usage of `hub_asset` on spoke `spoke_id`. Removes the storage key instead if
/// both the supplied and borrowed scaled totals are zero.
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
