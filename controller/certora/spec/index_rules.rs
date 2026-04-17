/// Index Safety & Monotonicity Rules
///
/// From CLAUDE.md:
///   - supply_index >= RAY -- violation = total supplier loss
///   - borrow_index >= RAY -- violation = interest calculation errors
///   - Indexes must be monotonically increasing
///     (except bad debt socialization for supply_index)
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::fp::Ray;

// ---------------------------------------------------------------------------
// Rule 1: Supply index never drops below RAY (1.0)
// ---------------------------------------------------------------------------

/// The supply index must always be >= RAY (10^27).
/// A supply index below 1.0 means suppliers have lost principal --
/// this should only happen during explicit bad debt socialization.
#[rule]
fn supply_index_gte_ray(e: Env, asset: Address) {
    // Assume the asset is initialized (has a config with non-zero threshold)
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);
    cvlr_assume!(config.liquidation_threshold_bps > 0);

    // Load the current market index for the asset
    let cache_entry = crate::storage::market_index::get_market_index(&e, &asset);

    cvlr_assert!(cache_entry.supply_index_ray >= RAY);
}

// ---------------------------------------------------------------------------
// Rule 2: Borrow index never drops below RAY (1.0)
// ---------------------------------------------------------------------------

/// The borrow index must always be >= RAY (10^27).
/// A borrow index below 1.0 would mean debt shrinks over time without repayment.
#[rule]
fn borrow_index_gte_ray(e: Env, asset: Address) {
    // Assume the asset is initialized (has a config with non-zero threshold)
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);
    cvlr_assume!(config.liquidation_threshold_bps > 0);

    let cache_entry = crate::storage::market_index::get_market_index(&e, &asset);

    cvlr_assert!(cache_entry.borrow_index_ray >= RAY);
}

// ---------------------------------------------------------------------------
// Rule 3: Borrow index monotonically increases after accrual
// ---------------------------------------------------------------------------

/// After interest accrual, the borrow index must not decrease.
/// Violation would mean debt is being erased without repayment.
#[rule]
fn borrow_index_monotonic_after_accrual(e: Env, asset: Address, caller: Address, account_id: u64) {
    // Capture index before
    let index_before = crate::storage::market_index::get_market_index(&e, &asset);
    let borrow_before = index_before.borrow_index_ray;

    // Trigger index update via any operation (supply triggers accrual)
    let amount: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    // Capture index after
    let index_after = crate::storage::market_index::get_market_index(&e, &asset);
    let borrow_after = index_after.borrow_index_ray;

    cvlr_assert!(borrow_after >= borrow_before);
}

// ---------------------------------------------------------------------------
// Rule 4: Supply index monotonically increases after accrual
//         (except during bad debt socialization)
// ---------------------------------------------------------------------------

/// After interest accrual (non-bad-debt path), supply index must not decrease.
#[rule]
fn supply_index_monotonic_after_accrual(e: Env, asset: Address, caller: Address, account_id: u64) {
    let index_before = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_before = index_before.supply_index_ray;

    let amount: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0);
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let index_after = crate::storage::market_index::get_market_index(&e, &asset);
    let supply_after = index_after.supply_index_ray;

    cvlr_assert!(supply_after >= supply_before);
}

// ---------------------------------------------------------------------------
// Rule 5: Indexes unchanged when no time has elapsed
// ---------------------------------------------------------------------------

/// When delta_time == 0, compound_interest returns RAY (1.0), so both the
/// borrow index and supply index must remain unchanged after an update.
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

    // delta_time = 0 => compound interest factor = RAY (exactly 1.0)
    let factor = common::rates::compound_interest(&e, Ray::from_raw(rate), 0);
    cvlr_assert!(factor == Ray::ONE);

    // Borrow index: old * RAY / RAY = old (identity)
    let new_borrow =
        common::rates::update_borrow_index(&e, Ray::from_raw(old_borrow_index), factor);
    cvlr_assert!(new_borrow.raw() == old_borrow_index);

    // Supply index: rewards_increase = 0 when no interest accrued => unchanged
    let new_supply = common::rates::update_supply_index(
        &e,
        Ray::from_raw(supplied),
        Ray::from_raw(old_supply_index),
        Ray::ZERO,
    );
    cvlr_assert!(new_supply.raw() == old_supply_index);
}

// ---------------------------------------------------------------------------
// Sanity
// ---------------------------------------------------------------------------

#[rule]
fn index_sanity(e: Env, asset: Address) {
    let idx = crate::storage::market_index::get_market_index(&e, &asset);
    cvlr_satisfy!(idx.supply_index_ray > 0 && idx.borrow_index_ray > 0);
}
