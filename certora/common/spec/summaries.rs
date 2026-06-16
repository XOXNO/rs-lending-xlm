//! Certora summaries for expensive common math.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::Env;

use crate::constants::MAX_BORROW_INDEX_RAY;
use crate::math::fp::Ray;
use crate::types::{MarketIndex, PoolSyncData};

/// Index accrual: indexes non-decreasing from input, borrow index <= MAX_BORROW_INDEX_RAY.
pub fn simulate_update_indexes_summary(
    _env: &Env,
    _current_timestamp: u64,
    sync: &PoolSyncData,
) -> MarketIndex {
    let borrow_in = sync.state.borrow_index_ray;
    let supply_in = sync.state.supply_index_ray;

    let borrow_out: i128 = nondet();
    let supply_out: i128 = nondet();

    cvlr_assume!(borrow_out >= borrow_in);
    cvlr_assume!(borrow_out <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(supply_out >= supply_in);
    cvlr_assume!(supply_out <= MAX_BORROW_INDEX_RAY);

    MarketIndex {
        supply_index: Ray::from(supply_out),
        borrow_index: Ray::from(borrow_out),
    }
}