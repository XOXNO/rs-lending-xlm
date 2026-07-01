use common::errors::{CollateralError, SpokeError};
use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketOracleConfigOption, SpokeAssetArgs, SpokeAssetConfig};
use soroban_sdk::{assert_with_error, Address, Env};

use crate::external::pool::fetch_pool_sync_data;
use crate::spoke::caps::{validate_spoke_caps_against_hub, validate_spoke_caps_against_usage};
use crate::{
    events::{RemoveSpokeAssetEvent, UpdateSpokeAssetEvent},
    storage,
};

pub fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    common::validation::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);

    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
        SpokeError::AssetAlreadyInSpoke
    );

    // The pool owns the market record; `fetch_pool_sync_data` reverts
    // `PoolNotInitialized` when the (hub, asset) market was never created, so the
    // asset must already have a created market before a spoke lists it.
    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &hub_asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        args.supply_cap,
        args.borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(
        env,
        args.supply_cap,
        hub.params.asset_decimals,
    );
    common::validation::require_cap_within_asset_domain(
        env,
        args.borrow_cap,
        hub.params.asset_decimals,
    );

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: false,
        frozen: false,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: resolve_spoke_oracle_override(
            env,
            &args.asset,
            hub.params.asset_decimals,
            &args.oracle_override,
        ),
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
        hub_id: args.hub_id,
    }
    .publish(env);
}

pub fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    common::validation::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &hub_asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        args.supply_cap,
        args.borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(
        env,
        args.supply_cap,
        hub.params.asset_decimals,
    );
    common::validation::require_cap_within_asset_domain(
        env,
        args.borrow_cap,
        hub.params.asset_decimals,
    );
    let usage = storage::get_spoke_usage(env, args.spoke_id, &hub_asset).unwrap_or_default();
    validate_spoke_caps_against_usage(
        env,
        &usage,
        args.supply_cap,
        args.borrow_cap,
        Ray::from(hub.state.supply_index),
        Ray::from(hub.state.borrow_index),
        hub.params.asset_decimals,
    );

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: false,
        frozen: false,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: resolve_spoke_oracle_override(
            env,
            &args.asset,
            hub.params.asset_decimals,
            &args.oracle_override,
        ),
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

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
            let mut cfg = cfg.clone();
            crate::config::oracle::validate_market_oracle_config(env, asset, &cfg);
            if cfg!(feature = "testing") && pool_decimals != 0 {
                cfg.asset_decimals = pool_decimals;
            }
            MarketOracleConfigOption::Some(cfg)
        }
    }
}

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
