use super::*;
use common::constants::RAY;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Env, Vec};

fn sample_asset_config(env: &Env) -> AssetConfigRaw {
    AssetConfigRaw {
        loan_to_value_bps: 7_500,
        liquidation_threshold_bps: 8_000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 100,
        is_collateralizable: true,
        is_borrowable: true,
        is_flashloanable: true,
        flashloan_fee_bps: 9,
        asset_decimals: 7,
        e_mode_categories: Vec::new(env),
    }
}

fn sample_market_params(asset: &Address, decimals: u32) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate_ray: RAY,
        base_borrow_rate_ray: 0,
        slope1_ray: RAY / 100,
        slope2_ray: RAY / 10,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: 8 * RAY / 10,
        max_utilization_ray: 95 * RAY / 100,
        reserve_factor_bps: 1_000,
        supply_cap: 5_000_000,
        borrow_cap: 1_000_000,
        is_flashloanable: false,
        flashloan_fee_bps: 0,
        asset_id: asset.clone(),
        asset_decimals: decimals,
    }
}

#[test]
#[should_panic]
fn test_validate_risk_bounds_rejects_threshold_above_bps() {
    let env = Env::default();
    validate_risk_bounds(&env, 5_000, 10_001, 100);
}

#[test]
#[should_panic]
fn test_validate_hub_caps_rejects_negative_supply_cap() {
    let env = Env::default();
    validate_hub_caps(&env, -1, 0);
}

#[test]
#[should_panic]
fn test_validate_asset_config_rejects_flashloan_fee_above_cap() {
    let env = Env::default();
    let mut cfg = sample_asset_config(&env);
    cfg.flashloan_fee_bps = (MAX_FLASHLOAN_FEE_BPS + 1) as u32;
    validate_asset_config(&env, &cfg);
}

#[test]
#[should_panic]
fn test_validate_position_limits_rejects_zero() {
    let env = Env::default();
    validate_position_limits(
        &env,
        &PositionLimits {
            max_supply_positions: 0,
            max_borrow_positions: 1,
        },
    );
}

#[test]
#[should_panic]
fn test_validate_position_limits_rejects_above_cap() {
    let env = Env::default();
    validate_position_limits(
        &env,
        &PositionLimits {
            max_supply_positions: 1,
            max_borrow_positions: POSITION_LIMIT_MAX + 1,
        },
    );
}

#[test]
#[should_panic]
fn test_validate_market_creation_rejects_wrong_asset_id() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let other = Address::generate(&env);
    let params = sample_market_params(&other, 7);
    let cfg = sample_asset_config(&env);
    validate_market_creation(&env, &asset, &params, &cfg, 7);
}

#[test]
#[should_panic]
fn test_validate_market_creation_rejects_decimals_out_of_range() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let params = sample_market_params(&asset, MAX_ASSET_DECIMALS + 1);
    let cfg = sample_asset_config(&env);
    validate_market_creation(&env, &asset, &params, &cfg, MAX_ASSET_DECIMALS + 1);
}
