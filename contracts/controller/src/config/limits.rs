//! Instance-level position-limit and minimum-borrow-collateral floor setters.

use common::errors::CollateralError;
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, Env};

use crate::events::{UpdateMinBorrowCollateralEvent, UpdatePositionLimitsEvent};
use crate::storage;

/// Stores the supply/borrow position limits and emits the update event.
pub(crate) fn set_position_limits(env: &Env, limits: PositionLimits) {
    storage::set_position_limits(env, &limits);
    UpdatePositionLimitsEvent {
        max_supply_positions: limits.max_supply_positions,
        max_borrow_positions: limits.max_borrow_positions,
    }
    .publish(env);
}

/// Stores the non-negative min-borrow-collateral USD WAD floor and emits the update event.
pub(crate) fn set_min_borrow_collateral_usd(env: &Env, floor_wad: i128) {
    assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
    storage::set_min_borrow_collateral_usd_wad(env, floor_wad);
    UpdateMinBorrowCollateralEvent {
        min_borrow_collateral_usd_wad: floor_wad,
    }
    .publish(env);
}

/// Returns the stored minimum-borrow-collateral USD WAD floor.
pub(crate) fn get_min_borrow_collateral_usd(env: &Env) -> i128 {
    storage::get_min_borrow_collateral_usd_wad(env)
}
