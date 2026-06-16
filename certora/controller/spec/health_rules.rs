/// Health factor invariant rules via inline unsummarised helper math.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::spec::health_ghost;
use common::math::fp::{Bps, Ray, Wad};

/// Sums borrow-side USD WAD by iterating `borrow_positions` without the summarised aggregate.
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

/// Sums liquidation-threshold-weighted collateral USD WAD from `supply_positions`.
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

/// After a successful borrow, weighted collateral covers total debt.
#[rule]
fn hf_safe_after_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

/// After a successful withdraw, weighted collateral still covers total debt.
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

/// Liquidation of a healthy account (weighted collateral >= debt) must revert.
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

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() >= pre_debt.raw());

    let mut payments: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset, debt_amount));

    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    cvlr_satisfy!(false);
}

/// Supply preserves weighted collateral >= total debt when it held pre-supply.
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

    cvlr_assert!(post_weighted.raw() >= post_debt.raw());
}

/// Borrow reaches a post-state (non-vacuous).
#[rule]
fn hf_borrow_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

/// Withdraw reaches a post-state (non-vacuous).
#[rule]
fn hf_withdraw_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    crate::spec::compat::withdraw_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

/// Scaled supply balance at `asset`, or 0 when absent.
fn scaled_supply_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .supply_positions
        .get(asset.clone())
        .map(|p| p.scaled_amount_ray)
        .unwrap_or(0)
}

/// Scaled borrow balance at `asset`, or 0 when absent.
fn scaled_borrow_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .borrow_positions
        .get(asset.clone())
        .map(|p| p.scaled_amount_ray)
        .unwrap_or(0)
}

/// After borrow, for any reserve: debt-free, safe-direction move, or solvency gate ran.
#[rule]
fn borrow_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    // The skolem reserve must be one the account actually holds (or the operated
    // asset). A fresh empty address makes the safe-direction disjunct trivially
    // true (0 >= 0 && 0 <= 0) and the ghost is never load-bearing.
    let reserve = cvlr_soroban::nondet_address();
    cvlr_assume!(
        reserve == asset
            || pre_account.supply_positions.contains_key(reserve.clone())
            || pre_account.borrow_positions.contains_key(reserve.clone())
    );
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

/// After withdraw, for any reserve: debt-free, safe-direction move, or solvency gate ran.
#[rule]
fn withdraw_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    // The skolem reserve must be one the account actually holds (or the operated
    // asset). A fresh empty address makes the safe-direction disjunct trivially
    // true (0 >= 0 && 0 <= 0) and the ghost is never load-bearing.
    let reserve = cvlr_soroban::nondet_address();
    cvlr_assume!(
        reserve == asset
            || pre_account.supply_positions.contains_key(reserve.clone())
            || pre_account.borrow_positions.contains_key(reserve.clone())
    );
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

/// Borrow path sets the solvency gate ghost (non-vacuous).
#[rule]
fn borrow_gated_sanity(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    health_ghost::reset();
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(health_ghost::get_checked());
}
