//! Instance-level position-limit and minimum-borrow-collateral floor setters.

use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, Env};

use crate::events::{UpdateMinBorrowCollateralEvent, UpdatePositionLimitsEvent};
use crate::storage;

/// Re-validates at execution (`1..=POSITION_LIMIT_MAX` per side) so a direct
/// owner call cannot store a zero cap (bricks a side) or an unbounded cap that
/// would outrun the liquidation budget; mirrors the governance propose gate.
pub(crate) fn set_position_limits(env: &Env, limits: PositionLimits) {
    let valid = 1..=POSITION_LIMIT_MAX;
    assert_with_error!(
        env,
        valid.contains(&limits.max_supply_positions)
            && valid.contains(&limits.max_borrow_positions),
        GenericError::InvalidPositionLimits
    );
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
