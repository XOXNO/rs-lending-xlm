//! Certora summaries for expensive common math.
//!
//! Summaries return values in the production domain so consumer proofs stay
//! sound; the bounds are proved against the real functions by dedicated lemmas
//! in `rates_rules`.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::Env;

use crate::constants::MAX_BORROW_INDEX_RAY;
use crate::math::fp::Ray;
use crate::types::{MarketIndex, PoolSyncData};

/// Summary for `rates::simulate_update_indexes`.
///
/// The real read-path accrual loop iterates an 8-term Taylor `compound_interest`
/// per chunk — the dominant nonlinearity (and the only loop) in every controller
/// pricing path. The summary returns a fresh `MarketIndex` that preserves the
/// load-bearing property: interest accrual never shrinks an index, and the
/// borrow index is clamped at `MAX_BORROW_INDEX_RAY`.
///
/// Soundness (proved against the real body in
/// `rates_rules::simulate_indexes_monotone_one_chunk` and `..._no_time_noop`):
///   * `borrow_index_out >= borrow_index_in` — borrow interest only grows.
///   * `supply_index_out >= supply_index_in` — supplier rewards only grow it;
///     the read path has no bad-debt write-down.
///   * `borrow_index_out <= MAX_BORROW_INDEX_RAY` — production clamp.
/// The matching `supply_index_out <= MAX_BORROW_INDEX_RAY` is a modeling bound
/// (the supply index tracks below the borrow ceiling); it is asserted over the
/// lemma's bounded domain but not over the pool's full multi-year history.
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
