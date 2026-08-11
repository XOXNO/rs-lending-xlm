//! Sets protocol-wide position count limits and the minimum borrow collateral
//! floor.

use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, Env};

use crate::events::{UpdateMinBorrowCollateralEvent, UpdatePositionLimitsEvent};
use crate::storage;

/// Sets the maximum number of concurrent supply and borrow positions and
/// publishes an `UpdatePositionLimitsEvent`. Panics if either limit is zero
/// or exceeds `POSITION_LIMIT_MAX`.
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

/// Sets the minimum USD-denominated collateral floor (WAD-scaled) required to
/// open a borrow position and publishes an `UpdateMinBorrowCollateralEvent`.
/// Panics if `floor_wad` is negative.
pub(crate) fn set_min_borrow_collateral_usd(env: &Env, floor_wad: i128) {
    assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
    storage::set_min_borrow_collateral_usd_wad(env, floor_wad);
    UpdateMinBorrowCollateralEvent {
        min_borrow_collateral_usd_wad: floor_wad,
    }
    .publish(env);
}
