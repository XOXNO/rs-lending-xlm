use super::protocol::{get_shared, increment_counter, set_shared};
use common::errors::{GenericError, SpokeError};
use common::types::{ControllerKey, HubAssetKey, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw};
use soroban_sdk::{panic_with_error, Env};

/// Issues the next spoke ID from the instance counter; fails on overflow.
pub(crate) fn increment_spoke_id(env: &Env) -> u32 {
    increment_counter(env, &ControllerKey::LastSpokeId)
}

/// Returns spoke config or fails with `SpokeNotFound`.
pub(crate) fn get_spoke(env: &Env, id: u32) -> SpokeConfig {
    try_get_spoke(env, id).unwrap_or_else(|| panic_with_error!(env, SpokeError::SpokeNotFound))
}

/// Returns shared persistent spoke config, renewing TTL when present.
pub(crate) fn try_get_spoke(env: &Env, id: u32) -> Option<SpokeConfig> {
    get_shared(env, &ControllerKey::Spoke(id))
}

/// Stores spoke config and renews shared persistent TTL.
pub(crate) fn set_spoke(env: &Env, id: u32, spoke: &SpokeConfig) {
    set_shared(env, &ControllerKey::Spoke(id), spoke);
}

/// Returns listed asset risk parameters and caps, renewing shared TTL if present.
pub(crate) fn get_spoke_asset(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    get_shared(env, &ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()))
}

/// Stores listed asset risk parameters and caps, renewing shared TTL.
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

/// Returns the listing's flags epoch, or 0 if no flag write happened yet.
pub(crate) fn get_spoke_flags_epoch(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) -> u64 {
    get_shared(
        env,
        &ControllerKey::SpokeFlagsEpoch(spoke_id, hub_asset.clone()),
    )
    .unwrap_or(0)
}

/// Increments the listing's flags epoch and renews shared TTL; fails on overflow.
pub(crate) fn bump_spoke_flags_epoch(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    let next = get_spoke_flags_epoch(env, spoke_id, hub_asset)
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    set_shared(
        env,
        &ControllerKey::SpokeFlagsEpoch(spoke_id, hub_asset.clone()),
        &next,
    );
}

/// Returns whether the spoke lists collateral below `MIN_BORROWABLE_ASSET_DECIMALS`.
pub(crate) fn is_whole_unit_spoke(env: &Env, spoke_id: u32) -> bool {
    get_shared(env, &ControllerKey::WholeUnitSpoke(spoke_id)).unwrap_or(false)
}

/// Marks the spoke as listing collateral below `MIN_BORROWABLE_ASSET_DECIMALS`.
pub(crate) fn mark_whole_unit_spoke(env: &Env, spoke_id: u32) {
    set_shared(env, &ControllerKey::WholeUnitSpoke(spoke_id), &true);
}

/// Deletes the listed asset config; does not modify usage or the flags epoch.
pub(crate) fn remove_spoke_asset(env: &Env, spoke_id: u32, hub_asset: &HubAssetKey) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset.clone()));
}

/// Returns scaled supply and debt usage in RAY, renewing shared TTL if present.
pub(crate) fn get_spoke_usage(
    env: &Env,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeUsageRaw> {
    get_shared(env, &ControllerKey::SpokeUsage(spoke_id, hub_asset.clone()))
}

/// Stores usage and renews shared TTL; deletes the row when both sides are zero.
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
