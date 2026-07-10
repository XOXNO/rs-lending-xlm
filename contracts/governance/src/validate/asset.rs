//! Asset, risk, limit, and token-shape validation.

use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::MarketParamsRaw;
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

// SAC decimal range for RAY/WAD conversions. Assets below 6 decimals can
// truncate small collateral toward zero in fixed-point valuation; floor
// lowered to 3 to admit lower-decimal RWA tokens (e.g. 5-decimal Spiko money
// market funds) at the cost of coarser dust-level precision for those assets.
const MIN_ASSET_DECIMALS: u32 = 3;
const MAX_ASSET_DECIMALS: u32 = 18;

pub(crate) fn validate_risk_bounds(env: &Env, ltv: u32, threshold: u32, bonus: u32) {
    common::validation::validate_risk_bounds(env, ltv, threshold, bonus);
}

pub(crate) fn validate_liquidation_fees(env: &Env, fees_bps: u32) {
    common::validation::validate_liquidation_fees(env, fees_bps);
}

pub(crate) fn validate_and_fetch_token_decimals(env: &Env, token: &Address) -> u32 {
    let token_client = token::Client::new(env, token);
    let Ok(Ok(decimals)) = token_client.try_decimals() else {
        panic_with_error!(env, GenericError::InvalidAsset);
    };
    assert_with_error!(
        env,
        matches!(token_client.try_symbol(), Ok(Ok(_))),
        GenericError::InvalidAsset
    );
    decimals
}

pub(crate) fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    let valid = 1..=POSITION_LIMIT_MAX;
    assert_with_error!(
        env,
        valid.contains(&limits.max_supply_positions)
            && valid.contains(&limits.max_borrow_positions),
        GenericError::InvalidPositionLimits
    );
}

pub(crate) fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
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

    params.verify(env);
}

pub(crate) fn validate_spoke_cap_args(env: &Env, supply_cap: i128, borrow_cap: i128) {
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
}

#[cfg(test)]
#[path = "../../tests/validate/asset.rs"]
mod tests;
