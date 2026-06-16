//! Supply and borrow index floor and monotonicity rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;

/// Supply index stays at or above the bad-debt socialization floor.
#[rule]
fn supply_index_above_floor(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);
    cvlr_assume!(config.liquidation_threshold_bps > 0);

    let cache_entry = crate::storage::market_index::get_market_index(&e, &asset);

    cvlr_assert!(cache_entry.supply_index.raw() >= SUPPLY_INDEX_FLOOR_RAW);
}

/// Borrow index stays at or above `RAY` (1.0).
#[rule]
fn borrow_index_gte_ray(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);
    cvlr_assume!(config.liquidation_threshold_bps > 0);

    let cache_entry = crate::storage::market_index::get_market_index(&e, &asset);

    cvlr_assert!(cache_entry.borrow_index.raw() >= RAY);
}

/// Borrow index does not decrease after accrual-triggering operations.
#[rule]
fn borrow_index_monotonic_after_accrual(e: Env, asset: Address, caller: Address, account_id: u64) {
    let index_before = crate::storage::market_index::get_market_index(&e, &asset);
    let borrow_before = index_before.borrow_index.raw();

    let amount: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let index_after = crate::storage::market_index::get_market_index(&e, &asset);
    let borrow_after = index_after.borrow_index.raw();

    cvlr_assert!(borrow_after >= borrow_before);
}

/// Supply index does not decrease after accrual-triggering operations.
#[rule]
fn supply_index_monotonic_after_accrual(e: Env, asset: Address, caller: Address, account_id: u64) {
    let index_before = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_before = index_before.supply_index.raw();

    let amount: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let index_after = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_after = index_after.supply_index.raw();

    cvlr_assert!(supply_after >= supply_before);
}

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