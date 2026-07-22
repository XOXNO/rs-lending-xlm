//! Supply and borrow index floor and monotonicity rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;

// Index floor/monotonicity via `get_market_index` is vacuous under
// `get_sync_data_summary` nondet; proved in pool integrity + common rates rules.

#[rule]
fn indexes_unchanged_when_no_time_elapsed(e: Env) {
    let old_borrow_index: i128 = cvlr::nondet::nondet();
    let old_supply_index: i128 = cvlr::nondet::nondet();
    let supplied: i128 = cvlr::nondet::nondet();
    let rate: i128 = cvlr::nondet::nondet();

    cvlr_assume!((RAY..=MAX_BORROW_INDEX_RAY).contains(&old_borrow_index));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=MAX_SUPPLY_INDEX_RAY).contains(&old_supply_index));
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
