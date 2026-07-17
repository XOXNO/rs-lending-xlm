//! Certora summaries for expensive common math.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::Env;

use crate::constants::MAX_BORROW_INDEX_RAY;
use crate::math::fp::Ray;
use crate::types::{MarketIndex, PoolSyncData};

/// Index accrual nondet: `borrow_out` in `[borrow_in, MAX_BORROW_INDEX_RAY]`,
/// `supply_out` in `[supply_in, borrow_out]`.
///
/// Soundness vs real `simulate_update_indexes`:
///   * borrow grows only by interest and is clamped at `MAX_BORROW_INDEX_RAY`
///   * supply grows only by supplier rewards (read path has no bad-debt write-down)
///   * supply tracks a fraction of borrow interest from shared `RAY`, so `supply <= borrow`
pub fn simulate_update_indexes_summary(
    _env: &Env,
    _current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    let borrow_in = sync.state.borrow_index;
    let supply_in = sync.state.supply_index;

    let borrow_out: i128 = nondet();
    let supply_out: i128 = nondet();

    cvlr_assume!(borrow_out >= borrow_in);
    cvlr_assume!(borrow_out <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(supply_out >= supply_in);
    cvlr_assume!(supply_out <= borrow_out);

    MarketIndex {
        supply_index: Ray::from(supply_out),
        borrow_index: Ray::from(borrow_out),
    }
}
