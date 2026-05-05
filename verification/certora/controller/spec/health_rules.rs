/// Health Factor Invariant Rules
///
/// Asserts safety properties (collateral coverage of debt) directly against
/// the unsummarised helper layer (`helpers::position_value`,
/// `helpers::calculate_ltv_collateral_wad`). The aggregate
/// `calculate_health_factor` / `calculate_account_totals` are summarised
/// (their summaries return independent nondets), so asserting on their return
/// values would not link the post-state to the operation under test. Pinning
/// to a single supply position and a single borrow position keeps the inline
/// iteration bounded, and `account_id = 1` removes a 64-bit symbolic
/// dimension per rule.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::WAD;
use common::fp::{Bps, Ray, Wad};

/// Computes the borrow-side USD WAD total by inline iteration over the
/// account's `borrow_positions` map. Mirrors the production borrow-side loop
/// in `helpers::calculate_account_totals` (controller/src/helpers/mod.rs:170)
/// but is *not* routed through the summarised aggregate.
fn inline_total_borrow_wad(
    env: &Env,
    cache: &mut crate::cache::ControllerCache,
    account_id: u64,
) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let mut total = Wad::ZERO;
    for asset in account.borrow_positions.keys() {
        let position = account.borrow_positions.get(asset.clone()).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = crate::helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.borrow_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        total += value;
    }
    total
}

/// Computes the liquidation-threshold-weighted collateral USD WAD by inline
/// iteration over `supply_positions`. Mirrors the production supply-side loop
/// in `helpers::calculate_account_totals` (controller/src/helpers/mod.rs:150)
/// without going through the summarised aggregate. Distinct from
/// `calculate_ltv_collateral_wad` which weights by `loan_to_value_bps`; this
/// one weights by `liquidation_threshold_bps` and is the correct quantity for
/// the HF safety inequality `weighted_collateral >= total_debt`.
fn inline_weighted_collateral_wad(
    env: &Env,
    cache: &mut crate::cache::ControllerCache,
    account_id: u64,
) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let mut weighted = Wad::ZERO;
    for asset in account.supply_positions.keys() {
        let position = account.supply_positions.get(asset.clone()).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = crate::helpers::position_value(
            env,
            Ray::from_raw(position.scaled_amount_ray),
            Ray::from_raw(market_index.supply_index_ray),
            Wad::from_raw(feed.price_wad),
        );
        weighted += crate::helpers::weighted_collateral(
            env,
            value,
            Bps::from_raw(position.liquidation_threshold_bps),
        );
    }
    weighted
}

// ---------------------------------------------------------------------------
// Rule 1: Health-factor safety after borrow (math-anchored)
// ---------------------------------------------------------------------------

/// After any successful borrow, the borrower's liquidation-threshold-weighted
/// collateral must cover the total debt. Computed inline against the
/// unsummarised helper layer instead of via `calculate_health_factor_for`
/// (which is summarised and returns an independent nondet).
#[rule]
fn hf_safe_after_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Bound the iteration to a single position on each side. Combined with
    // `account_id = 1`, this collapses the production map iterations to one
    // step.
    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    // Safety: weighted collateral covers total debt.
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

// ---------------------------------------------------------------------------
// Rule 2: Health-factor safety after withdraw (math-anchored)
// ---------------------------------------------------------------------------

/// After any successful withdraw, the withdrawer's weighted collateral must
/// still cover total debt. Same math-anchored approach as Rule 1.
#[rule]
fn hf_safe_after_withdraw(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset, amount);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

// ---------------------------------------------------------------------------
// Rule 3: Liquidation requires unhealthy account (math-anchored)
// ---------------------------------------------------------------------------

/// Liquidation must only be possible when the account is unhealthy
/// (`weighted_collateral < total_debt`). The pre-state safety condition is
/// asserted as an assumption against the inline math; if liquidation reaches
/// the post-state on a healthy account, the rule fails via `cvlr_satisfy!(false)`.
#[rule]
fn liquidation_requires_unhealthy_account(
    e: Env,
    liquidator: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(debt_amount > 0 && debt_amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    // Pre-state: account is healthy (weighted collateral >= total debt).
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() >= pre_debt.raw());

    let mut payments: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset, debt_amount));

    // Liquidating a healthy account must revert.
    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    // Reaching this line means liquidation succeeded on a healthy account.
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 4: Supply preserves the safety inequality (math-anchored)
// ---------------------------------------------------------------------------

/// Supplying additional collateral must preserve the
/// `weighted_collateral >= total_debt` safety inequality if it held before.
/// Asserted directly on the inline math instead of comparing two summarised
/// HF nondets.
#[rule]
fn supply_cannot_decrease_hf(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() >= pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 = crate::cache::ControllerCache::new(&e, false);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    // Supply does not introduce new debt, so debt must not have grown; combined
    // with the pre-state safety inequality, the post-state remains safe.
    cvlr_assert!(post_weighted.raw() >= post_debt.raw());
}

// ---------------------------------------------------------------------------
// Sanity rules (reachability checks -- ensures rules aren't vacuously true)
// ---------------------------------------------------------------------------

#[rule]
fn hf_borrow_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

#[rule]
fn hf_withdraw_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    crate::spec::compat::withdraw_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}
