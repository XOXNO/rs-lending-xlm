//! Spoke-asset listing helpers: add, edit, remove, and tighten-only flags.

use common::errors::{CollateralError, SpokeError};
use common::types::{HubAssetKey, PoolSyncData, SpokeAssetArgs, SpokeAssetConfig};
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

/// Lists a hub-asset on a spoke after validating risk bounds, caps, and any oracle override.
/// New listings are refused on a deprecated spoke; edits stay allowed.
pub(crate) fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    let hub_asset = validate_spoke_asset_args(env, args);
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
        SpokeError::AssetAlreadyInSpoke
    );

    load_market_and_validate_caps(env, args, &hub_asset);
    let config = build_spoke_asset_config(args);
    store_spoke_asset(env, args, &hub_asset, config);
}

pub(crate) fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    let hub_asset = validate_spoke_asset_args(env, args);
    storage::get_spoke(env, args.spoke_id);
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    load_market_and_validate_caps(env, args, &hub_asset);
    let config = build_spoke_asset_config(args);
    store_spoke_asset(env, args, &hub_asset, config);
}

fn validate_spoke_asset_args(env: &Env, args: &SpokeAssetArgs) -> HubAssetKey {
    common_validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    common_validate_liquidation_fees(env, args.liquidation_fees);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );

    HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    }
}

fn load_market_and_validate_caps(
    env: &Env,
    args: &SpokeAssetArgs,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    // The pool owns the market record; this reverts `PoolNotInitialized` when
    // `(hub, asset)` was never created.
    let market = fetch_pool_sync_data(env, &storage::get_pool(env), hub_asset);
    // These caps feed `Ray::from_asset`; reject overflow-prone configs here.
    require_cap_within_asset_domain(env, args.supply_cap, market.params.asset_decimals);
    require_cap_within_asset_domain(env, args.borrow_cap, market.params.asset_decimals);
    market
}

/// Resolves the stored listing from validated arguments.
fn build_spoke_asset_config(args: &SpokeAssetArgs) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: args.paused,
        frozen: args.frozen,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
    }
}

/// Persists the listing and publishes its resolved snapshot.
fn store_spoke_asset(
    env: &Env,
    args: &SpokeAssetArgs,
    hub_asset: &HubAssetKey,
    config: SpokeAssetConfig,
) {
    storage::set_spoke_asset(env, args.spoke_id, hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
        hub_id: args.hub_id,
    }
    .publish(env);
}

/// Tightens only `paused`/`frozen` on an existing listing (works on deprecated
/// spokes). Clearing a flag reverts `SpokeAssetFlagRelaxation`; reopen via
/// timelocked `edit_asset_in_spoke`.
pub(crate) fn set_spoke_asset_flags(
    env: &Env,
    spoke_id: u32,
    hub_asset: HubAssetKey,
    paused: bool,
    frozen: bool,
) {
    let mut config = storage::get_spoke_asset(env, spoke_id, &hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
    assert_with_error!(
        env,
        (paused || !config.paused) && (frozen || !config.frozen),
        SpokeError::SpokeAssetFlagRelaxation
    );
    config.paused = paused;
    config.frozen = frozen;
    storage::set_spoke_asset(env, spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: hub_asset.asset,
        config,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}

/// Unlists a hub-asset from a spoke. Requires zero usage so a live position's
/// listing always exists; wind a listing down with `frozen` first.
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
