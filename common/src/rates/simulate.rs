//! The protocol's interest-accrual step, and the read-only projection built
//! on it.
//!
//! [`accrue_step`] is the single implementation of one compounding step. Both
//! accrual paths run it: the pool's mutating `interest::global_sync` and the
//! no-write [`simulate_update_indexes`] here, so the view and the mutator agree
//! bit-for-bit.

use soroban_sdk::{panic_with_error, Env};

use crate::constants::MAX_MARKET_VALUE_RAY;
use crate::errors::GenericError;
use crate::math::fp::Ray;
use crate::types::{MarketIndex, MarketParams, PoolState, PoolSyncData};

use crate::rates::compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
use crate::rates::curve::{calculate_borrow_rate, utilization};
use crate::rates::index::{
    calculate_supplier_rewards, protocol_fee_shares, supply_index_reward_shortfall,
    update_borrow_index, update_supply_index,
};

/// Outcome of one compounding step: the new indexes, plus the protocol
/// revenue owed for the step.
pub struct AccrualStep {
    /// Borrow index after compounding this step's interest.
    pub borrow_index: Ray,
    /// Supply index after distributing this step's supplier rewards.
    pub supply_index: Ray,
    /// Scaled supply shares owed to the protocol for this step, valued at
    /// `supply_index`. `Ray::ZERO` when the step accrued no fee.
    ///
    /// The pool books them as revenue and adds them to total supply; the
    /// simulator adds them to its running supply only. Either way they must be
    /// added to total supply before the next step, because the next step's
    /// utilization reads it.
    pub revenue_shares: Ray,
    /// `true` when [`MAX_MARKET_VALUE_RAY`] held this step's borrow index
    /// below what the rate implies, so borrowers accrued less interest.
    pub value_capped: bool,
}

/// Runs one compounding step of length `delta_ms` over a market snapshot.
///
/// Reads the snapshot and writes nothing.
///
/// `supplied` must already include revenue shares minted by earlier steps,
/// since utilization and the reward split are both computed against it.
/// `borrowed` does not change across steps.
///
/// Rounding is directed so the residual favours the protocol: whatever the
/// supply index cannot absorb comes back from
/// [`supply_index_reward_shortfall`] and is booked as revenue rather than
/// silently dropped.
///
/// The step never overflows on non-negative totals (INV-IDX-06). Interest
/// raises total debt value and total supply value by the same amount, so the
/// borrow index grows only as far as the larger total has room below
/// [`MAX_MARKET_VALUE_RAY`]. With no room left, or with a total that does not
/// fit `i128` at all, both indexes stay where they are and no revenue is
/// booked. Both totals then stay at or below the ceiling once the caller adds
/// the revenue shares.
///
/// Panics with `GenericError::MathOverflow` if a share total or an index is
/// negative.
pub fn accrue_step(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    supplied: Ray,
    borrow_index: Ray,
    supply_index: Ray,
    delta_ms: u64,
) -> AccrualStep {
    let (Some(borrowed_original), Some(supplied_original)) = (
        market_value(env, borrowed, borrow_index),
        market_value(env, supplied, supply_index),
    ) else {
        return AccrualStep {
            borrow_index,
            supply_index,
            revenue_shares: Ray::ZERO,
            value_capped: borrowed != Ray::ZERO,
        };
    };
    let room = value_room(env, borrowed_original, supplied_original);

    let util = utilization(env, borrowed_original, supplied_original);
    let borrow_rate = calculate_borrow_rate(env, util, params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let grown_borrow_index = update_borrow_index(env, borrow_index, interest_factor);
    let new_borrow_index = cap_borrow_index(env, borrowed, borrow_index, grown_borrow_index, room);

    let (supplier_rewards, protocol_fee) =
        calculate_supplier_rewards(env, params, borrowed, new_borrow_index, borrow_index);

    let new_supply_index = update_supply_index(env, supplied, supply_index, supplier_rewards);
    let supplier_shortfall = supply_index_reward_shortfall(
        env,
        supplied,
        supply_index,
        new_supply_index,
        supplier_rewards,
    );

    let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
    // Shares are valued at the new supply index, which the caller stores for
    // this step.
    let revenue_shares = if protocol_reward == Ray::ZERO {
        Ray::ZERO
    } else {
        protocol_fee_shares(env, protocol_reward, new_supply_index, supplied)
    };

    AccrualStep {
        borrow_index: new_borrow_index,
        supply_index: new_supply_index,
        revenue_shares,
        value_capped: new_borrow_index != grown_borrow_index,
    }
}

/// Values `scaled` shares at `index`, rounding half up. Returns `None` if the
/// value does not fit `i128`.
///
/// Panics with `GenericError::MathOverflow` if `scaled` or `index` is
/// negative.
fn market_value(env: &Env, scaled: Ray, index: Ray) -> Option<Ray> {
    if scaled < Ray::ZERO || index < Ray::ZERO {
        panic_with_error!(env, GenericError::MathOverflow);
    }
    scaled.try_mul(env, index)
}

/// Returns the value one step's interest may still add before the larger of
/// `debt_value` and `supply_value` reaches [`MAX_MARKET_VALUE_RAY`]. Zero at
/// or above the ceiling.
fn value_room(env: &Env, debt_value: Ray, supply_value: Ray) -> Ray {
    let ceiling = Ray::from(MAX_MARKET_VALUE_RAY);
    let top = debt_value.max(supply_value);
    if top >= ceiling {
        return Ray::ZERO;
    }
    ceiling.checked_sub(env, top)
}

/// Lowers `grown_index` so the debt value of `borrowed` shares rises by at
/// most `room` from its value at `old_index`. The allowed index growth,
/// `room / borrowed`, rounds down.
///
/// Returns `grown_index` unchanged when there is no debt or no growth. The
/// result is never below `old_index`.
fn cap_borrow_index(env: &Env, borrowed: Ray, old_index: Ray, grown_index: Ray, room: Ray) -> Ray {
    if borrowed == Ray::ZERO || grown_index <= old_index {
        return grown_index;
    }
    let max_growth = room.div_floor_saturating(env, borrowed);
    if grown_index.checked_sub(env, old_index) <= max_growth {
        return grown_index;
    }
    old_index.checked_add(env, max_growth)
}

/// Computes the borrow and supply indexes for `sync`'s pool state as of
/// `current_timestamp` (milliseconds), without persisting the result.
pub fn simulate_update_indexes(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    simulate_update_indexes_dispatch(env, current_timestamp, sync)
}

/// Dispatches to [`simulate_update_indexes_body`].
#[cfg(not(feature = "certora"))]
fn simulate_update_indexes_dispatch(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    simulate_update_indexes_body(env, current_timestamp, sync)
}

#[cfg(feature = "certora")]
cvlr_soroban_macros::apply_summary!(
    crate::spec::summaries::simulate_update_indexes_summary,
    /// Dispatches to [`simulate_update_indexes_body`].
    pub(crate) fn simulate_update_indexes_dispatch(
        env: &Env,
        current_timestamp: u64,
        sync: &PoolSyncData,
    ) -> MarketIndex {
        simulate_update_indexes_body(env, current_timestamp, sync)
    }
);

/// Simulates borrow and supply index updates for the elapsed interval
/// between `state.last_timestamp` and `current_timestamp`, without
/// persisting state.
///
/// Returns the stored indexes unchanged if no time has elapsed. Otherwise
/// runs [`accrue_step`] over chunks of at most `MAX_COMPOUND_DELTA_MS` and
/// adds each step's revenue shares to the running supply.
pub(crate) fn simulate_update_indexes_body(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    let state = PoolState::from(&sync.state);
    let total_delta_ms = current_timestamp.saturating_sub(state.last_timestamp);

    if total_delta_ms == 0 {
        return MarketIndex {
            supply_index: state.supply_index,
            borrow_index: state.borrow_index,
        };
    }

    let params = MarketParams::from(&sync.params);

    let mut supplied = state.supplied;
    let mut borrow_index = state.borrow_index;
    let mut supply_index = state.supply_index;

    let mut remaining = total_delta_ms;
    while remaining > 0 {
        let chunk = remaining.min(MAX_COMPOUND_DELTA_MS);
        let step = accrue_step(
            env,
            &params,
            state.borrowed,
            supplied,
            borrow_index,
            supply_index,
            chunk,
        );

        borrow_index = step.borrow_index;
        supply_index = step.supply_index;
        supplied = supplied.checked_add(env, step.revenue_shares);

        remaining -= chunk;
    }

    MarketIndex {
        supply_index,
        borrow_index,
    }
}

#[cfg(test)]
#[path = "../../tests/rates/simulate.rs"]
mod tests;
