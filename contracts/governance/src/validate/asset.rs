//! Validation helpers for asset onboarding: token decimals and symbol checks,
//! per-spoke position limits, market creation parameters, and spoke cap arguments.

use common::constants::{MAX_ASSET_DECIMALS, MIN_ASSET_DECIMALS, POSITION_LIMIT_MAX};
use common::errors::{CollateralError, GenericError};
use common::types::{MarketParamsRaw, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, token, Address, Env};

/// Fetches `token`'s decimals via a cross-contract call and returns them.
/// Panics with `GenericError::InvalidAsset` if the decimals call fails, or if the
/// token does not also expose a symbol via `try_symbol`.
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

/// Panics with `GenericError::InvalidPositionLimits` unless both
/// `max_supply_positions` and `max_borrow_positions` fall within `1..=POSITION_LIMIT_MAX`.
pub(crate) fn validate_position_limits(env: &Env, limits: &PositionLimits) {
    let valid = 1..=POSITION_LIMIT_MAX;
    assert_with_error!(
        env,
        valid.contains(&limits.max_supply_positions)
            && valid.contains(&limits.max_borrow_positions),
        GenericError::InvalidPositionLimits
    );
}

/// Validates market creation parameters against `asset` and the already-fetched
/// `token_decimals`: `params.asset_id` must equal `asset`, `params.asset_decimals`
/// must equal `token_decimals` and fall within `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`,
/// and delegates further checks to `params.verify`. Panics with `GenericError::WrongToken`
/// or `GenericError::InvalidAsset` on the respective mismatch.
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

/// Panics with `CollateralError::InvalidBorrowParams` unless both `supply_cap`
/// and `borrow_cap` are non-negative.
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
