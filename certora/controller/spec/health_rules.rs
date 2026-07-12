/// Health factor invariant rules via inline unsummarised helper math.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::spec::health_ghost;
use crate::types::HubAssetKey;
use common::math::fp::{Bps, Ray, Wad};

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

/// Sums borrow-side USD WAD by iterating `borrow_positions` without the summarised aggregate.
fn inline_total_borrow_wad(env: &Env, cache: &mut crate::context::Cache, account_id: u64) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let mut total = Wad::ZERO;
    for hub_asset in account.borrow_positions.keys() {
        let position = account.borrow_positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value(
            env,
            Ray::from(position.scaled_amount),
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
    cache: &mut crate::context::Cache,
    account_id: u64,
) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let mut weighted = Wad::ZERO;
    for hub_asset in account.supply_positions.keys() {
        let position = account.supply_positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value(
            env,
            Ray::from(position.scaled_amount),
            market_index.supply_index,
            feed.price,
        );
        weighted +=
            crate::risk::weighted_collateral(env, value, Bps::from(position.liquidation_threshold));
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

    let mut cache = crate::context::Cache::new(&e);
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

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

/// A healthy account (weighted collateral >= debt) has HF >= 1 under the gate's
/// own `div_floor` formula — exactly the value the liquidation gate
/// (`assert hf < ONE`) rejects, so it cannot be liquidated. Proven on the real
/// unsummarised valuation; the gate's own HF is summarised, so the link to
/// `process_liquidation` is by the gate's definition, not executed here.
#[rule]
fn liquidation_requires_unhealthy_account(e: Env) {
    let account_id: u64 = 1;
    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(debt.raw() > 0);
    cvlr_assume!(weighted.raw() >= debt.raw());

    // weighted >= debt ⇒ floor(weighted / debt) >= 1; the boundary weighted == debt
    // gives exactly 1, so the gate never misclassifies a healthy account.
    let hf = weighted.div_floor(&e, debt);
    cvlr_assert!(hf.raw() >= WAD);
}

/// Supply preserves weighted collateral >= total debt when it held pre-supply.
#[rule]
fn supply_cannot_decrease_hf(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() >= pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 = crate::context::Cache::new(&e);
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
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

/// Scaled borrow balance at `asset`, or 0 when absent.
fn scaled_borrow_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .borrow_positions
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
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
            || pre_account.supply_positions.contains_key(hub0(&reserve))
            || pre_account.borrow_positions.contains_key(hub0(&reserve))
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
            || pre_account.supply_positions.contains_key(hub0(&reserve))
            || pre_account.borrow_positions.contains_key(hub0(&reserve))
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

// Strategy HF gates + unhealthy-only-improves + threshold-downgrade
// (ported from the pre-hub-refactor certora-hardening branch).

/// A freshly opened leverage position must satisfy the safety inequality.
#[rule]
fn hf_safe_after_multiply(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
    flash_amount: i128,
    steps: crate::types::StrategySwap,
) {
    cvlr_assume!(flash_amount > 0 && flash_amount <= WAD * 1000);
    cvlr_assume!(collateral_token != debt_token);

    let account_id = crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        0, // default spoke
        collateral_token,
        flash_amount,
        debt_token,
        1, // PositionMode::Multiply
        steps,
    );

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

/// swap_debt lands inside the safety inequality.
#[rule]
fn hf_safe_after_swap_debt(
    e: Env,
    caller: Address,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    swap: soroban_sdk::Bytes,
) {
    let account_id: u64 = 1;
    cvlr_assume!(new_debt_amount > 0 && new_debt_amount <= WAD * 1000);
    cvlr_assume!(existing_debt_token != new_debt_token);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        hub0(&existing_debt_token),
        new_debt_amount,
        hub0(&new_debt_token),
        swap,
    );

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

/// swap_collateral lands inside the safety inequality.
#[rule]
fn hf_safe_after_swap_collateral(
    e: Env,
    caller: Address,
    current_collateral: Address,
    amount: i128,
    new_collateral: Address,
    swap: soroban_sdk::Bytes,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(current_collateral != new_collateral);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        hub0(&current_collateral),
        amount,
        hub0(&new_collateral),
        swap,
    );

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

#[rule]
fn hf_multiply_sanity(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
    flash_amount: i128,
    steps: crate::types::StrategySwap,
) {
    cvlr_assume!(flash_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::compat::multiply_minimal(
        e,
        caller,
        0,
        collateral_token,
        flash_amount,
        debt_token,
        1,
        steps,
    );
    cvlr_satisfy!(true);
}

/// On an unhealthy account, repay must not grow debt and must not shrink
/// weighted collateral — division-free "HF below 1 can only increase".
#[rule]
fn unhealthy_repay_only_improves(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw()); // account is unhealthy

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 = crate::context::Cache::new(&e);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}

/// Supply leg of the only-improves family.
#[rule]
fn unhealthy_supply_only_improves(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 = crate::context::Cache::new(&e);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}

/// `apply_liquidation_threshold` only lowers a stored threshold on an
/// indebted account when the simulated HF clears the 1.05 buffer: any supply
/// that actually lowered the stored threshold leaves the account safe.
#[rule]
fn threshold_downgrade_implies_account_safe(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() == 1);
    cvlr_assume!(pre_account.borrow_positions.len() == 1);
    let pre_position = pre_account.supply_positions.get(hub0(&asset));
    cvlr_assume!(pre_position.is_some());
    let pre_lt = pre_position.unwrap().liquidation_threshold;

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let post_account = crate::storage::get_account(&e, account_id);
    let post_position = post_account.supply_positions.get(hub0(&asset));
    cvlr_assume!(post_position.is_some());
    let post_lt = post_position.unwrap().liquidation_threshold;

    // Only audit executions where the stored threshold actually dropped.
    cvlr_assume!(post_lt < pre_lt);

    let mut cache = crate::context::Cache::new(&e);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
