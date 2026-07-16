//! Spoke-asset listing setters: add, edit, and remove a hub-asset on a spoke,
//! with risk-bound, cap-domain, and oracle-override validation.

use common::errors::{CollateralError, GenericError, SpokeError};
use common::types::{
    HubAssetKey, MarketOracleConfigOption, PoolSyncData, SpokeAssetArgs, SpokeAssetConfig,
};
use common::validation::{
    require_cap_within_asset_domain, validate_liquidation_fees as common_validate_liquidation_fees,
    validate_risk_bounds as common_validate_risk_bounds,
};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::config::oracle::validate_market_oracle_config;
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

    let market = load_market_and_validate_caps(env, args, &hub_asset);
    let config = build_spoke_asset_config(env, args, market.params.asset_decimals);
    store_spoke_asset(env, args, &hub_asset, config);
}

/// Updates a spoke-asset listing. Works on deprecated spokes so live listings
/// stay manageable while usage drains. Caps may sit below current usage:
/// enforcement is entry-time only, so a lower cap just blocks new exposure
/// until exits drain usage under it.
pub(crate) fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    let hub_asset = validate_spoke_asset_args(env, args);
    storage::get_spoke(env, args.spoke_id);
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    let market = load_market_and_validate_caps(env, args, &hub_asset);
    let config = build_spoke_asset_config(env, args, market.params.asset_decimals);
    store_spoke_asset(env, args, &hub_asset, config);
}

/// Validates common risk bounds and returns the listing's hub coordinate.
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

/// Loads the pool market and validates both caps against its decimal domain.
fn load_market_and_validate_caps(
    env: &Env,
    args: &SpokeAssetArgs,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    // The pool owns the market record; this reverts `PoolNotInitialized` when
    // `(hub, asset)` was never created.
    let market = fetch_pool_sync_data(env, &storage::get_pool(env), hub_asset);
    // These caps feed `Ray::from_asset`; reject overflow-prone configs here.
    require_cap_within_asset_domain(
        env,
        args.supply_cap,
        market.params.asset_decimals,
    );
    require_cap_within_asset_domain(
        env,
        args.borrow_cap,
        market.params.asset_decimals,
    );
    market
}

/// Resolves the stored listing from validated arguments and pool decimals.
fn build_spoke_asset_config(
    env: &Env,
    args: &SpokeAssetArgs,
    pool_decimals: u32,
) -> SpokeAssetConfig {
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
        oracle_override: resolve_spoke_oracle_override(
            env,
            &args.asset,
            pool_decimals,
            &args.oracle_override,
        ),
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

/// Validates a per-spoke oracle override.
fn resolve_spoke_oracle_override(
    env: &Env,
    asset: &Address,
    pool_decimals: u32,
    input: &MarketOracleConfigOption,
) -> MarketOracleConfigOption {
    match input {
        MarketOracleConfigOption::None => MarketOracleConfigOption::None,
        MarketOracleConfigOption::Some(cfg) => {
            validate_market_oracle_config(env, asset, cfg);
            // Override decimals feed `usd_value_wad` for every position on the
            // spoke; a mismatch against the pool market's decimals mis-scales
            // valuations by powers of ten.
            assert_with_error!(
                env,
                cfg.asset_decimals == pool_decimals,
                GenericError::InvalidAsset
            );
            MarketOracleConfigOption::Some(cfg.clone())
        }
    }
}

/// Sets only the `paused`/`frozen` flags on an existing listing, preserving
/// every other field. The guardian incident path: no risk params, caps, or
/// override travel with it, and it works on deprecated spokes. Containment
/// only — each flag may tighten (`false -> true`) or stay put; clearing one
/// is risk-loosening and must ride the timelocked `EditAssetInSpoke`, which
/// also works on deprecated spokes.
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
