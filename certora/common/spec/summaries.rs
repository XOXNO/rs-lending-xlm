//! Certora summaries for expensive common math.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::Env;

use crate::constants::MAX_BORROW_INDEX_RAY;
use crate::math::fp::Ray;
use crate::types::{MarketIndex, PoolSyncData};

/// Index accrual model: indexes non-decreasing from input; the borrow index is
/// clamped at `MAX_BORROW_INDEX_RAY` (production `update_borrow_index` clamp) and
/// the supply index stays at or below the borrow index.
///
/// Soundness of the bounds (against the real `simulate_update_indexes`):
///   * `borrow_out >= borrow_in`, `borrow_out <= MAX_BORROW_INDEX_RAY` — borrow
///     interest only grows the index and `update_borrow_index` clamps it.
///   * `supply_out >= supply_in` — supplier rewards only grow the supply index
///     (the read path has no bad-debt write-down).
///   * `supply_out <= borrow_out` — the supply index accrues a fraction (the
///     supplier-reward share) of the same interest the borrow index accrues in
///     full, starting from a common `RAY`, so it provably tracks at or below the
///     borrow index. This replaces the earlier unproven `<= MAX` modeling bound
///     and keeps the result overflow-safe without over-constraining.
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
    cvlr_assume!(supply_out <= borrow_out);

    MarketIndex {
        supply_index: Ray::from(supply_out),
        borrow_index: Ray::from(borrow_out),
    }
}
