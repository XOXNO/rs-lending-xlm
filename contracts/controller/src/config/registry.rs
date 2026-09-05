use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, Address, Env};

use crate::events::{
    ApproveBlendPoolEvent, UpdateAccumulatorEvent, UpdateMinBorrowCollateralEvent,
    UpdatePositionLimitsEvent, UpdatePriceAggregatorEvent, UpdateSwapAggregatorEvent,
};
use crate::storage;

/// Stores the swap aggregator and emits its address.
pub(crate) fn set_swap_aggregator(env: &Env, addr: Address) {
    storage::set_swap_aggregator(env, &addr);
    UpdateSwapAggregatorEvent {
        swap_aggregator: addr,
    }
    .publish(env);
}

/// Stores the price aggregator and emits its address.
pub(crate) fn set_price_aggregator(env: &Env, addr: Address) {
    storage::set_price_aggregator(env, &addr);
    UpdatePriceAggregatorEvent {
        price_aggregator: addr,
    }
    .publish(env);
}

/// Stores the revenue accumulator and emits its address.
pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

/// Updates the Blend pool allowlist and emits the approval state.
pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}

/// Stores and emits position limits, each in `1..=POSITION_LIMIT_MAX`.
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

/// Stores and emits the nonnegative LTV-weighted borrow collateral floor
/// in USD WAD. Zero disables the floor.
pub(crate) fn set_min_borrow_collateral_usd(env: &Env, floor_wad: i128) {
    assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
    storage::set_min_borrow_collateral_usd_wad(env, floor_wad);
    UpdateMinBorrowCollateralEvent {
        min_borrow_collateral_usd_wad: floor_wad,
    }
    .publish(env);
}
