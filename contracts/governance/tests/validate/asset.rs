use super::*;
use common::constants::RAY;
use controller_interface::types::MarketOracleConfigOption;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

fn sample_asset_config() -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: 7_500,
        liquidation_threshold: 8_000,
        liquidation_bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: MarketOracleConfigOption::None,
    }
}

fn sample_market_params(asset: &Address, decimals: u32) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: 0,
        slope1: RAY / 100,
        slope2: RAY / 10,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: 8 * RAY / 10,
        max_utilization: 95 * RAY / 100,
        reserve_factor: 1_000,
        supply_cap: 5_000_000,
        borrow_cap: 1_000_000,
        is_flashloanable: false,
        flashloan_fee: 0,
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
    let cfg = sample_asset_config();
    validate_market_creation(&env, &asset, &params, &cfg, 7);
}

#[test]
#[should_panic]
fn test_validate_market_creation_rejects_decimals_out_of_range() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let params = sample_market_params(&asset, MAX_ASSET_DECIMALS + 1);
    let cfg = sample_asset_config();
    validate_market_creation(&env, &asset, &params, &cfg, MAX_ASSET_DECIMALS + 1);
}
