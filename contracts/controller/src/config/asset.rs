use common::errors::{CollateralError, SpokeError};
use common::types::{HubAssetKey, SpokeAssetArgs, SpokeAssetConfig};
use common::validation::{
    require_cap_within_asset_domain, validate_liquidation_fees as common_validate_liquidation_fees,
    validate_risk_bounds as common_validate_risk_bounds,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::external::pool::fetch_pool_sync_data;
use crate::{
    events::{RemoveSpokeAssetEvent, UpdateSpokeAssetEvent},
    storage,
};

enum SpokeAssetMutation {
    Add,
    Edit,
}

/// Lists a new market in an active spoke after validating risk parameters and caps.
pub(crate) fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    upsert_spoke_asset(env, args, SpokeAssetMutation::Add);
}

/// Updates a listed market after validating risk parameters and caps.
/// Allows deprecated spokes so existing positions remain configurable.
/// Flags may stay or tighten; clearing one reverts.
pub(crate) fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    upsert_spoke_asset(env, args, SpokeAssetMutation::Edit);
}

/// Checks registration state and risk bounds, validates caps against pool
/// decimals, then stores and emits the asset config. Advances the flags epoch
/// on a new listing or a flag change.
fn upsert_spoke_asset(env: &Env, args: &SpokeAssetArgs, mutation: SpokeAssetMutation) {
    common_validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    common_validate_liquidation_fees(env, args.liquidation_fees);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );

    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    let stored = match mutation {
        SpokeAssetMutation::Add => {
            let spoke = storage::get_spoke(env, args.spoke_id);
            assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
            assert_with_error!(
                env,
                storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
                SpokeError::AssetAlreadyInSpoke
            );
            None
        }
        SpokeAssetMutation::Edit => {
            storage::get_spoke(env, args.spoke_id);
            let stored = storage::get_spoke_asset(env, args.spoke_id, &hub_asset)
                .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
            require_flag_ratchet(env, &stored, args.paused, args.frozen, args.no_seize);
            Some(stored)
        }
    };

    let market = fetch_pool_sync_data(env, &storage::get_pool(env), &hub_asset);
    require_cap_within_asset_domain(env, args.supply_cap, market.params.asset_decimals);
    require_cap_within_asset_domain(env, args.borrow_cap, market.params.asset_decimals);

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: args.paused,
        frozen: args.frozen,
        no_seize: args.no_seize,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);
    if stored.is_none_or(|stored| flags(&stored) != flags(&config)) {
        storage::bump_spoke_flags_epoch(env, args.spoke_id, &hub_asset);
    }

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
        hub_id: args.hub_id,
    }
    .publish(env);
}

fn flags(config: &SpokeAssetConfig) -> (bool, bool, bool) {
    (config.paused, config.frozen, config.no_seize)
}

/// Tightens paused, frozen, and no-seize flags on a listed asset, advances
/// its flags epoch, and emits the new config. Rejects any true-to-false transition.
pub(crate) fn set_spoke_asset_flags(
    env: &Env,
    spoke_id: u32,
    hub_asset: HubAssetKey,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    let config = storage::get_spoke_asset(env, spoke_id, &hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
    require_flag_ratchet(env, &config, paused, frozen, no_seize);
    write_spoke_asset_flags(env, spoke_id, hub_asset, config, paused, frozen, no_seize);
}

/// Writes any paused, frozen, and no-seize combination on a listed asset when
/// `expected_epoch` equals its flags epoch, then advances the epoch and emits
/// the new config.
pub(crate) fn relax_spoke_asset_flags(
    env: &Env,
    spoke_id: u32,
    hub_asset: HubAssetKey,
    expected_epoch: u64,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    let config = storage::get_spoke_asset(env, spoke_id, &hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
    assert_with_error!(
        env,
        storage::get_spoke_flags_epoch(env, spoke_id, &hub_asset) == expected_epoch,
        SpokeError::SpokeFlagsEpochMismatch
    );
    write_spoke_asset_flags(env, spoke_id, hub_asset, config, paused, frozen, no_seize);
}

/// Stores the three flags, advances the flags epoch, and emits the new config.
fn write_spoke_asset_flags(
    env: &Env,
    spoke_id: u32,
    hub_asset: HubAssetKey,
    mut config: SpokeAssetConfig,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    config.paused = paused;
    config.frozen = frozen;
    config.no_seize = no_seize;
    storage::set_spoke_asset(env, spoke_id, &hub_asset, &config);
    storage::bump_spoke_flags_epoch(env, spoke_id, &hub_asset);

    UpdateSpokeAssetEvent {
        asset: hub_asset.asset,
        config,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}

/// Allows only unchanged or tightened flags. Clearing flags requires the
/// timelocked `relax_spoke_asset_flags` path (ADR-0007, INV-AUTH-04).
fn require_flag_ratchet(
    env: &Env,
    config: &SpokeAssetConfig,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    assert_with_error!(
        env,
        (paused || !config.paused) && (frozen || !config.frozen) && (no_seize || !config.no_seize),
        SpokeError::SpokeAssetFlagRelaxation
    );
}

/// Removes and emits a listed asset only when both scaled usage amounts are zero.
pub(crate) fn remove_asset_from_spoke(env: &Env, hub_asset: HubAssetKey, spoke_id: u32) {
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );
    let usage = storage::get_spoke_usage(env, spoke_id, &hub_asset).unwrap_or_default();
    assert_with_error!(
        env,
        usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0,
        SpokeError::SpokeAssetInUse
    );

    storage::remove_spoke_asset(env, spoke_id, &hub_asset);

    RemoveSpokeAssetEvent {
        asset: hub_asset.asset,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}

#[cfg(test)]
#[path = "../../tests/config/asset_flags.rs"]
mod tests;
