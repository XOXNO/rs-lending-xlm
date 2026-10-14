//! The protocol's interest-accrual step, and the read-only projection built
//! on it.
//!
//! [`accrue_step`] is the single implementation of one compounding step. Both
//! accrual paths run it: the pool's mutating `interest::global_sync` and the
//! no-write [`simulate_update_indexes`] here, so the view and the mutator agree
//! bit-for-bit.

use soroban_sdk::Env;

use crate::constants::{MAX_MARKET_VALUE_RAY, RAY};
use crate::math::fp::Ray;
use crate::math::fp_core::mul_div_floor_saturating;
use crate::types::{MarketIndex, MarketParams, PoolState, PoolSyncData};

use crate::rates::compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
use crate::rates::curve::{calculate_borrow_rate, utilization};
use crate::rates::index::{
    calculate_supplier_rewards, protocol_fee_shares, supply_index_reward_shortfall,
    update_borrow_index, update_supply_index,
};
use crate::rates::scaling::scaled_to_original;

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
    /// `true` when [`MAX_MARKET_VALUE_RAY`] held this step's borrower interest
    /// below what the rate implies.
    pub value_capped: bool,
}

/// Raw ray units of value kept below [`MAX_MARKET_VALUE_RAY`] for one step's
/// rounding: the half-up totals, the floored supply index and the floored
/// revenue shares together add under three raw units beyond the capped
/// interest.
const ACCRUAL_ROUNDING_SLACK_RAW: i128 = 3;

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
/// Interest raises total debt value and total supply value by the same
/// amount, so the step caps it at the room both totals have below
/// [`MAX_MARKET_VALUE_RAY`]. With no room left, the step leaves both indexes
/// unchanged and books no revenue.
pub fn accrue_step(
    env: &Env,
    params: &MarketParams,
    borrowed: Ray,
    supplied: Ray,
    borrow_index: Ray,
    supply_index: Ray,
    delta_ms: u64,
) -> AccrualStep {
    let headroom = market_value_headroom(env, borrowed, supplied, borrow_index, supply_index);
    if headroom == 0 {
        return AccrualStep {
            borrow_index,
            supply_index,
            revenue_shares: Ray::ZERO,
            value_capped: borrowed != Ray::ZERO,
        };
    }

    let borrowed_original = scaled_to_original(env, borrowed, borrow_index);
    let supplied_original = scaled_to_original(env, supplied, supply_index);
    let util = utilization(env, borrowed_original, supplied_original);
    let borrow_rate = calculate_borrow_rate(env, util, params);
    let interest_factor = compound_interest(env, borrow_rate, delta_ms);

    let grown_borrow_index = update_borrow_index(env, borrow_index, interest_factor);
    let (new_borrow_index, value_capped) =
        cap_borrow_index_growth(env, borrowed, borrow_index, grown_borrow_index, headroom);

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
        value_capped,
    }
}

/// Raw ray value by which one step's interest may still raise the market
/// totals: [`MAX_MARKET_VALUE_RAY`] less [`ACCRUAL_ROUNDING_SLACK_RAW`] less the
/// larger of total supply value and total debt value, floored at zero.
///
/// The totals are floored and saturate, so a total that does not fit `i128`
/// leaves no room instead of panicking.
fn market_value_headroom(
    env: &Env,
    borrowed: Ray,
    supplied: Ray,
    borrow_index: Ray,
    supply_index: Ray,
) -> i128 {
    let supplied_value = mul_div_floor_saturating(env, supplied.raw(), supply_index.raw(), RAY);
    let borrowed_value = mul_div_floor_saturating(env, borrowed.raw(), borrow_index.raw(), RAY);
    (MAX_MARKET_VALUE_RAY - ACCRUAL_ROUNDING_SLACK_RAW)
        .saturating_sub(supplied_value.max(borrowed_value))
        .max(0)
}

/// Lowers `grown_index` so `borrowed` debt value grows by at most `headroom`
/// raw ray from `old_index`. Returns the index and whether the cap engaged.
///
/// The capped index never falls below `old_index`.
fn cap_borrow_index_growth(
    env: &Env,
    borrowed: Ray,
    old_index: Ray,
    grown_index: Ray,
    headroom: i128,
) -> (Ray, bool) {
    if borrowed == Ray::ZERO {
        return (grown_index, false);
    }
    let max_growth = mul_div_floor_saturating(env, headroom, RAY, borrowed.raw());
    let ceiling = old_index.raw().saturating_add(max_growth);
    if grown_index.raw() <= ceiling {
        (grown_index, false)
    } else {
        (Ray::from(ceiling), true)
    }
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
