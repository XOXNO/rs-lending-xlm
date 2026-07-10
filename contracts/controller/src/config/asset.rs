//! Spoke-asset listing setters: add, edit, and remove a hub-asset on a spoke,
//! with risk-bound, cap-domain, and oracle-override validation.

use common::errors::{CollateralError, GenericError, SpokeError};
use common::math::fp::Ray;
use common::types::{
    HubAssetKey, MarketOracleConfigOption, PoolSyncData, SpokeAssetArgs, SpokeAssetConfig,
};
use soroban_sdk::{assert_with_error, Address, Env};

use crate::config::oracle::validate_market_oracle_config;
use crate::external::pool::fetch_pool_sync_data;
use crate::spoke::caps::validate_spoke_caps_against_usage;
use crate::{
    events::{RemoveSpokeAssetEvent, UpdateSpokeAssetEvent},
    storage,
};

/// Lists a hub-asset on a spoke after validating risk bounds, caps, and any oracle override.
pub fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    let hub_asset = validate_spoke_asset_args(env, args);
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
        SpokeError::AssetAlreadyInSpoke
    );

    let market = load_market_and_validate_caps(env, args, &hub_asset);
    let config = build_spoke_asset_config(env, args, market.params.asset_decimals);
    store_spoke_asset(env, args, &hub_asset, config);
}

/// Updates a spoke-asset listing, rejecting caps that fall below current spoke usage.
pub fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    let hub_asset = validate_spoke_asset_args(env, args);
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    let market = load_market_and_validate_caps(env, args, &hub_asset);
    let usage = storage::get_spoke_usage(env, args.spoke_id, &hub_asset).unwrap_or_default();
    validate_spoke_caps_against_usage(
        env,
        &usage,
        args.supply_cap,
        args.borrow_cap,
        Ray::from(market.state.supply_index),
        Ray::from(market.state.borrow_index),
        market.params.asset_decimals,
    );

    let config = build_spoke_asset_config(env, args, market.params.asset_decimals);
    store_spoke_asset(env, args, &hub_asset, config);
}

/// Validates common risk bounds and returns the listing's hub coordinate.
fn validate_spoke_asset_args(env: &Env, args: &SpokeAssetArgs) -> HubAssetKey {
    common::validation::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    common::validation::validate_liquidation_fees(env, args.liquidation_fees);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);

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
    common::validation::require_cap_within_asset_domain(
        env,
        args.supply_cap,
        market.params.asset_decimals,
    );
    common::validation::require_cap_within_asset_domain(
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

/// Unlists a hub-asset from a spoke, reverting when it is not listed.
pub fn remove_asset_from_spoke(env: &Env, hub_asset: HubAssetKey, spoke_id: u32) {
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    storage::remove_spoke_asset(env, spoke_id, &hub_asset);

    RemoveSpokeAssetEvent {
        asset: hub_asset.asset,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}
