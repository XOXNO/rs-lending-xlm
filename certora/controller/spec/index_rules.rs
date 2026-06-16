//! Supply and borrow index floor and monotonicity rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::RAY;
use common::math::fp::Ray;

// Index floor/monotonicity rules that read `get_market_index` were removed: under
// the certora harness that resolves to `get_sync_data_summary`, an independent
// nondet per call, so "floor" asserts were tautologies (re-asserting the summary's
// own assume) and "monotonic across op" asserts compared two unrelated nondet
// draws (not entailed). Those invariants are proved where the real math runs:
// the supply-index floor in `pool/spec/integrity_rules.rs`
// (bad_debt_socialization_keeps_supply_index_above_floor) and index monotonicity
// in `common/spec/rates_rules.rs` (update_borrow/supply_index_monotonic_*).

/// Zero elapsed time leaves both indexes unchanged.
#[rule]
fn indexes_unchanged_when_no_time_elapsed(e: Env) {
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let old_supply_index: i128 = cvlr::nondet::nondet();
    let supplied: i128 = cvlr::nondet::nondet();
    let rate: i128 = cvlr::nondet::nondet();

    cvlr_assume!(old_borrow_index >= RAY);
    cvlr_assume!(old_supply_index >= RAY);
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(rate >= 0);

    let factor = common::rates::compound_interest(&e, Ray::from(rate), 0);
    cvlr_assert!(factor == Ray::ONE);

    let new_borrow = common::rates::update_borrow_index(&e, Ray::from(old_borrow_index), factor);
    cvlr_assert!(new_borrow.raw() == old_borrow_index);

    let new_supply = common::rates::update_supply_index(
        &e,
        Ray::from(supplied),
        Ray::from(old_supply_index),
        Ray::ZERO,
    );
    cvlr_assert!(new_supply.raw() == old_supply_index);
}

#[rule]
fn index_sanity(e: Env, asset: Address) {
    let idx = crate::storage::market_index::get_market_index(&e, &asset);
    cvlr_satisfy!(idx.supply_index.raw() > 0 && idx.borrow_index.raw() > 0);
}
