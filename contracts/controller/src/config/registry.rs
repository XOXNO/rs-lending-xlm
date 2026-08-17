use common::constants::POSITION_LIMIT_MAX;
use common::errors::{CollateralError, GenericError};
use common::types::PositionLimits;
use soroban_sdk::{assert_with_error, Address, Env};

use crate::events::{
    ApproveBlendPoolEvent, UpdateAccumulatorEvent, UpdateMinBorrowCollateralEvent,
    UpdatePositionLimitsEvent, UpdatePriceAggregatorEvent, UpdateSwapAggregatorEvent,
};
use crate::storage;

/// Sets the swap aggregator address and publishes an
/// `UpdateSwapAggregatorEvent`.
pub(crate) fn set_swap_aggregator(env: &Env, addr: Address) {
    storage::set_swap_aggregator(env, &addr);
    UpdateSwapAggregatorEvent {
        swap_aggregator: addr,
    }
    .publish(env);
}

/// Sets the price aggregator address and publishes an
/// `UpdatePriceAggregatorEvent`.
pub(crate) fn set_price_aggregator(env: &Env, addr: Address) {
    storage::set_price_aggregator(env, &addr);
    UpdatePriceAggregatorEvent {
        price_aggregator: addr,
    }
    .publish(env);
}

/// Sets the accumulator address and publishes an `UpdateAccumulatorEvent`.
pub(crate) fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

/// Returns whether `pool` is on the Blend pool allowlist, defaulting to
/// `false` if unset.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: Address) -> bool {
    storage::is_blend_pool_approved(env, &pool)
}

/// Adds or removes `pool` from the Blend pool allowlist and publishes an
/// `ApproveBlendPoolEvent`.
pub(crate) fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}

/// Sets the maximum supply and borrow position counts and publishes an
/// `UpdatePositionLimitsEvent`. Panics if either limit is zero or exceeds
/// `POSITION_LIMIT_MAX`.
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

/// Sets the WAD-scaled USD collateral floor required to open a borrow
/// position and publishes an `UpdateMinBorrowCollateralEvent`. Panics if
/// `floor_wad` is negative.
pub(crate) fn set_min_borrow_collateral_usd(env: &Env, floor_wad: i128) {
    assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
    storage::set_min_borrow_collateral_usd_wad(env, floor_wad);
    UpdateMinBorrowCollateralEvent {
        min_borrow_collateral_usd_wad: floor_wad,
    }
    .publish(env);
}
