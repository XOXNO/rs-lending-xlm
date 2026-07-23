/// Health factor invariant rules via inline unsummarised helper math.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::constants::WAD;
use crate::spec::health_ghost;
use crate::types::HubAssetKey;
use common::math::fp::{Bps, Ray, Wad};

/// Primary-hub coordinate for `asset`.
fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

/// Primes prices and pool indexes for the inline valuation helpers. This
/// mirrors production risk entry points and prevents missing-cache vacuity.
fn prime_position_inputs(cache: &mut crate::context::Cache, keys: &Vec<HubAssetKey>) {
    cache.load_markets(keys);
}

/// Sums borrow-side USD WAD by iterating `borrow_positions` without the summarised aggregate.
fn inline_total_borrow_wad(env: &Env, cache: &mut crate::context::Cache, account_id: u64) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    let keys = account.borrow_positions.keys();
    prime_position_inputs(cache, &keys);
    let mut total = Wad::ZERO;
    for hub_asset in keys.iter() {
        let position = account.borrow_positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value_ceil(
            env,
            Ray::from(position.scaled_amount),
            market_index.borrow_index,
            feed.price,
        );
        total.checked_add_assign(env, value);
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
    let keys = account.supply_positions.keys();
    prime_position_inputs(cache, &keys);
    let mut weighted = Wad::ZERO;
    for hub_asset in keys.iter() {
        let position = account.supply_positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value_floor(
            env,
            Ray::from(position.scaled_amount),
            market_index.supply_index,
            feed.price,
        );
        weighted.checked_add_assign(
            env,
            crate::risk::weighted_collateral(env, value, Bps::from(position.liquidation_threshold)),
        );
    }
    weighted
}

#[rule]
fn supply_preserves_frozen_valuation_health_components(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    // Reuse the pre-state price/index cache. This proves the position mutation
    // direction at one valuation snapshot, not temporal oracle/index behavior.
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
    cvlr_assert!(post_debt.raw() == pre_debt.raw());
}

#[rule]
fn hf_borrow_sanity(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

#[rule]
fn hf_withdraw_sanity(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 2,
    );
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

#[rule]
fn borrow_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    // Skolem reserve must be held or be the operated asset (empty addr trivializes).
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

#[rule]
fn withdraw_safe_or_health_gated(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    // Skolem reserve must be held or be the operated asset (empty addr trivializes).
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

#[rule]
fn borrow_gated_sanity(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );
    health_ghost::reset();
    crate::spec::compat::borrow_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(health_ghost::get_checked());
}

#[rule]
fn hf_multiply_sanity(e: Env, caller: Address, collateral_token: Address, debt_token: Address) {
    let steps = cvlr_soroban::nondet_bytes1();
    let flash_amount = WAD;
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::compat::multiply_minimal(
        e,
        caller,
        crate::spec::fixture::SPOKE_ID,
        collateral_token,
        flash_amount,
        debt_token,
        1,
        steps,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn unhealthy_repay_improves_frozen_valuation_components(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw()); // account is unhealthy

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}

#[rule]
fn unhealthy_supply_improves_frozen_valuation_components(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache = crate::context::Cache::new(&e);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}
