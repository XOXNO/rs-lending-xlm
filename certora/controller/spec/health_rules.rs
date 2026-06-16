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

use crate::constants::WAD;
use crate::spec::health_ghost;
use common::math::fp::{Bps, Ray, Wad};

/// Computes the borrow-side USD WAD total by inline iteration over the
/// account's `borrow_positions` map. Mirrors the production borrow-side loop
/// in `helpers::calculate_account_totals` (controller/src/helpers/mod.rs:170)
/// but is *not* routed through the summarised aggregate.
fn inline_total_borrow_wad(env: &Env, cache: &mut crate::cache::Cache, account_id: u64) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let mut total = Wad::ZERO;
    for asset in account.borrow_positions.keys() {
        let position = account.borrow_positions.get(asset.clone()).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = crate::helpers::position_value(
            env,
            Ray::from(position.scaled_amount_ray),
            market_index.borrow_index,
            feed.price,
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
    cache: &mut crate::cache::Cache,
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
            Ray::from(position.scaled_amount_ray),
            market_index.supply_index,
            feed.price,
        );
        weighted += crate::helpers::weighted_collateral(
            env,
            value,
            Bps::from(position.liquidation_threshold_bps),
        );
    }
    weighted
}
// Rule 1: Health-factor safety after borrow (math-anchored)

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

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    // Safety: weighted collateral covers total debt.
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
// Rule 2: Health-factor safety after withdraw (math-anchored)

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

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
// Rule 3: Liquidation requires unhealthy account (math-anchored)

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
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
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
// Rule 4: Supply preserves the safety inequality (math-anchored)

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

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() >= pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    // Supply does not introduce new debt, so debt must not have grown; combined
    // with the pre-state safety inequality, the post-state remains safe.
    cvlr_assert!(post_weighted.raw() >= post_debt.raw());
}
// Sanity rules (reachability checks -- ensures rules aren't vacuously true)

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
// Blend-style "safe-direction OR health-gated" rules
//
// Faithful port of Certora/Blend `health_rules.rs::user_health_execute_submit`
// (which uses a `GHOST_CHECKED` flag + a Skolem index over the position map):
// for an ARBITRARY reserve (the Skolem `asset`, drawn nondeterministically),
// after a risk-changing op the borrower either ended debt-free, OR moved in a
// safe direction at that reserve (collateral non-decreasing AND debt
// non-increasing), OR the production solvency gate executed
// (`health_ghost::get_checked()`). Proving it for an arbitrary Skolem asset
// generalises the property to every reserve from one proof — the coverage our
// math-anchored rules above lack (they pin a single position).
//
// Unlike the aggregate-inequality rules, these read only the Skolem reserve's
// scaled balances (no per-position sums), so they carry no fixed-point
// nonlinearity of their own; cost comes only from the operation path.

/// Scaled supply (collateral) balance at `asset`, or 0 when absent.
fn scaled_supply_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .supply_positions
        .get(asset.clone())
        .map(|p| p.scaled_amount_ray)
        .unwrap_or(0)
}

/// Scaled borrow (debt) balance at `asset`, or 0 when absent.
fn scaled_borrow_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .borrow_positions
        .get(asset.clone())
        .map(|p| p.scaled_amount_ray)
        .unwrap_or(0)
}

/// After any borrow, for an arbitrary reserve the borrower is either debt-free,
/// moved safely at that reserve, or had the solvency gate run.
#[rule]
fn borrow_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    // Bound the account to <= 1 position per side so the real gate's portfolio
    // walk stays bounded; the Skolem `reserve` still generalises the property
    // to any asset.
    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let reserve = cvlr_soroban::nondet_address();
    let pre_coll = scaled_supply_at(&e, account_id, &reserve);
    let pre_debt = scaled_borrow_at(&e, account_id, &reserve);

    health_ghost::reset();
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let post_account = crate::storage::get_account(&e, account_id);
    let has_debt = !post_account.borrow_positions.is_empty();
    let post_coll = scaled_supply_at(&e, account_id, &reserve);
    let post_debt = scaled_borrow_at(&e, account_id, &reserve);

    cvlr_assert!(
        health_ghost::get_checked()
            || !has_debt
            || (post_coll >= pre_coll && post_debt <= pre_debt)
    );
}

/// After any withdraw, for an arbitrary reserve the withdrawer is either
/// debt-free, moved safely at that reserve, or had the solvency gate run.
#[rule]
fn withdraw_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let reserve = cvlr_soroban::nondet_address();
    let pre_coll = scaled_supply_at(&e, account_id, &reserve);
    let pre_debt = scaled_borrow_at(&e, account_id, &reserve);

    health_ghost::reset();
    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset, amount);

    let post_account = crate::storage::get_account(&e, account_id);
    let has_debt = !post_account.borrow_positions.is_empty();
    let post_coll = scaled_supply_at(&e, account_id, &reserve);
    let post_debt = scaled_borrow_at(&e, account_id, &reserve);

    cvlr_assert!(
        health_ghost::get_checked()
            || !has_debt
            || (post_coll >= pre_coll && post_debt <= pre_debt)
    );
}

#[rule]
fn borrow_gated_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    health_ghost::reset();
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(health_ghost::get_checked());
}
