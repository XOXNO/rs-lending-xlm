//! Read-path index accrual: same arithmetic as the pool's mutating accrual,
//! without touching storage.

use soroban_sdk::Env;

use crate::math::fp::Ray;
use crate::types::{MarketIndex, MarketParams, PoolState, PoolSyncData};

use crate::rates::compound::{compound_interest, MAX_COMPOUND_DELTA_MS};
use crate::rates::curve::{calculate_borrow_rate, utilization};
use crate::rates::index::{
    calculate_supplier_rewards, protocol_fee_shares, supply_index_reward_shortfall,
    update_borrow_index, update_supply_index,
};
use crate::rates::scaling::scaled_to_original;

/// Simulates index accrual without mutating pool storage.
/// Recomputes utilization and protocol revenue for each accrual chunk.
///
/// Three functions implement this one operation, and the split is a
/// verification seam: `simulate_update_indexes` is the stable public name,
/// `simulate_update_indexes_dispatch` is swapped by `cfg` for a Certora
/// summary, and `simulate_update_indexes_body` holds the real accrual loop.
/// The prover cannot expand the full Taylor series across an unbounded chunk
/// count, so under `--features certora` the dispatch is replaced by a monotone
/// nondeterministic summary.
pub fn simulate_update_indexes(
    env: &Env,
    current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    simulate_update_indexes_dispatch(env, current_timestamp, sync)
}

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
    pub(crate) fn simulate_update_indexes_dispatch(
        env: &Env,
        current_timestamp: u64,
        sync: &PoolSyncData,
    ) -> MarketIndex {
        simulate_update_indexes_body(env, current_timestamp, sync)
    }
);

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
        let chunk = core::cmp::min(remaining, MAX_COMPOUND_DELTA_MS);

        let borrowed_original = scaled_to_original(env, state.borrowed, borrow_index);
        let supplied_original = scaled_to_original(env, supplied, supply_index);
        let util = utilization(env, borrowed_original, supplied_original);
        let borrow_rate = calculate_borrow_rate(env, util, &params);
        let interest_factor = compound_interest(env, borrow_rate, chunk);

        let new_borrow_index = update_borrow_index(env, borrow_index, interest_factor);

        let (supplier_rewards, protocol_fee) = calculate_supplier_rewards(
            env,
            &params,
            state.borrowed,
            new_borrow_index,
            borrow_index,
        );

        let old_supply_index = supply_index;
        supply_index = update_supply_index(env, supplied, old_supply_index, supplier_rewards);
        let supplier_shortfall = supply_index_reward_shortfall(
            env,
            supplied,
            old_supply_index,
            supply_index,
            supplier_rewards,
        );
        borrow_index = new_borrow_index;

        // Reserve fee plus virtual-offset shortfall mint scaled supply and feed
        // the next chunk's utilization exactly like mutating pool accrual.
        let protocol_reward = protocol_fee.checked_add(env, supplier_shortfall);
        if protocol_reward != Ray::ZERO {
            // Overflow-safe: a floored supply index can push the share count past
            // i128; `protocol_fee_shares` saturates and caps to remaining headroom.
            let fee_scaled = protocol_fee_shares(env, protocol_reward, supply_index, supplied);
            supplied = supplied.checked_add(env, fee_scaled);
        }

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
