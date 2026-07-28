//! Asset, position-limit, market-params, and SAC decimal checks for proposals.

use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::{MarketParamsRaw, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

/// Inclusive SAC decimal bounds for RAY/WAD conversions.
const MIN_ASSET_DECIMALS: u32 = 3;
const MAX_ASSET_DECIMALS: u32 = 18;

/// Reads decimals from the token contract and confirms `symbol` is callable.
///
/// # Errors
/// * [`GenericError::InvalidAsset`] — not a readable SAC.
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

/// Rejects position limits outside `1..=POSITION_LIMIT_MAX`.
///
/// # Errors
/// * [`GenericError::InvalidPositionLimits`] — out of range.
pub(crate) fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    let valid = 1..=POSITION_LIMIT_MAX;
    assert_with_error!(
        env,
        valid.contains(&limits.max_supply_positions)
            && valid.contains(&limits.max_borrow_positions),
        GenericError::InvalidPositionLimits
    );
}

/// Full gate for listing a market: asset id match, live decimals match, decimal
/// range, and `MarketParamsRaw::verify`.
///
/// # Errors
/// * [`GenericError::WrongToken`] — params asset id mismatch.
/// * [`GenericError::InvalidAsset`] — decimal mismatch or out of range.
/// * Params verify errors from the raw market params type.
pub(crate) fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParamsRaw,
    token_decimals: u32,
) {
    assert_with_error!(env, params.asset_id == *asset, GenericError::WrongToken);
    assert_with_error!(
        env,
        params.asset_decimals == token_decimals,
        GenericError::InvalidAsset
    );

    assert_with_error!(
        env,
        (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&params.asset_decimals),
        GenericError::InvalidAsset
    );

    params.verify(env);
}

/// Rejects negative supply or borrow caps.
///
/// # Errors
/// * [`CollateralError::InvalidBorrowParams`] — either cap is negative.
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
