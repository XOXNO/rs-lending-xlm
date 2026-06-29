//! Risk-bound, asset-config, position-limit, and market-creation validation,
//! plus the live token-shape probe.

use common::constants::{BPS, POSITION_LIMIT_MAX};
use common::errors::{CollateralError, GenericError};
use common::types::MarketParamsRaw;
use controller_interface::types::{PositionLimits, SpokeAssetConfig};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

// SAC decimal range for RAY/WAD conversions. Assets below 6 decimals can
// truncate small collateral toward zero in fixed-point valuation.
const MIN_ASSET_DECIMALS: u32 = 6;
const MAX_ASSET_DECIMALS: u32 = 18;

pub(crate) fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    // Uses the shared controller risk-bound validator.
    common::validation::validate_risk_bounds(env, ltv, threshold, bonus);
}

pub(crate) fn validate_and_fetch_token_decimals(env: &Env, token: &Address) -> u32 {
    let token_client = token::Client::new(env, token);
    let decimals = match token_client.try_decimals() {
        Ok(Ok(d)) => d,
        _ => panic_with_error!(env, GenericError::InvalidAsset),
    };
    assert_with_error!(
        env,
        matches!(token_client.try_symbol(), Ok(Ok(_))),
        GenericError::InvalidAsset
    );
    decimals
}

pub(crate) fn validate_asset_config(env: &Env, config: &SpokeAssetConfig) {
    validate_risk_bounds(
        env,
        config.loan_to_value,
        config.liquidation_threshold,
        config.liquidation_bonus,
    );

    assert_with_error!(
        env,
        i128::from(config.liquidation_fees) <= BPS,
        CollateralError::InvalidLiqThreshold
    );
}

pub(crate) fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > POSITION_LIMIT_MAX
        || limits.max_borrow_positions > POSITION_LIMIT_MAX
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
    }
}

pub(crate) fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    config: &SpokeAssetConfig,
    _token_decimals: u32,
) {
    assert_with_error!(env, params.asset_id == *asset, GenericError::WrongToken);
    #[cfg(not(feature = "testing"))]
    assert_with_error!(
        env,
        params.asset_decimals == _token_decimals,
        GenericError::InvalidAsset
    );

    assert_with_error!(
        env,
        (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&params.asset_decimals),
        GenericError::InvalidAsset
    );

    validate_asset_config(env, config);
    params.verify(env);
}

pub(crate) fn validate_hub_caps(env: &Env, supply_cap: i128, borrow_cap: i128) {
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
}

#[cfg(test)]
#[path = "../../tests/validate/asset.rs"]
mod tests;
