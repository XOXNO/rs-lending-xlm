use super::*;
use common::constants::{BPS, RAY};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

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
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset.clone(),
        asset_decimals: decimals,
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn validate_risk_bounds_rejects_threshold_above_bps() {
    let env = Env::default();
    validate_risk_bounds(&env, 5_000, 10_001, 100);
}

#[test]
#[should_panic(expected = "Error(Contract, #113)")]
fn validate_liquidation_fees_rejects_above_bps() {
    let env = Env::default();
    validate_liquidation_fees(&env, BPS as u32 + 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn validate_spoke_cap_args_rejects_negative_supply_cap() {
    let env = Env::default();
    validate_spoke_cap_args(&env, -1, 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #36)")]
fn validate_position_limits_rejects_zero() {
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
#[should_panic(expected = "Error(Contract, #36)")]
fn validate_position_limits_rejects_above_cap() {
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
#[should_panic(expected = "Error(Contract, #8)")]
fn validate_market_creation_rejects_wrong_asset_id() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let other = Address::generate(&env);
    let params = sample_market_params(&other, 7);
    validate_market_creation(&env, &asset, &params, 7);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn validate_market_creation_rejects_decimals_out_of_range() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let params = sample_market_params(&asset, MAX_ASSET_DECIMALS + 1);
    validate_market_creation(&env, &asset, &params, MAX_ASSET_DECIMALS + 1);
}
