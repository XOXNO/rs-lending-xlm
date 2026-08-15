use controller_interface::ControllerInterface;
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Map, Vec};

use crate::constants::WAD;
use crate::spec::health_ghost;
use crate::types::{AccountPositionRaw, DebtPositionRaw, HubAssetKey};
use common::math::fp::{Bps, Ray, Wad};

fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

fn prime_position_inputs(cache: &mut crate::context::Cache, keys: &Vec<HubAssetKey>) {
    cache.load_markets(keys);
}

/// Total debt (WAD) of an arbitrary debt book, valued with `cache`'s prices
/// and borrow indexes. Mirrors production's `total_debt` leg of
/// `calculate_account_risk_totals` (ceiling-rounded per position).
fn total_borrow_wad_of(
    env: &Env,
    cache: &mut crate::context::Cache,
    positions: &Map<HubAssetKey, DebtPositionRaw>,
) -> Wad {
    let keys = positions.keys();
    prime_position_inputs(cache, &keys);
    let mut total = Wad::ZERO;
    for hub_asset in keys.iter() {
        let position = positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value_ceil(
            env,
            Ray::from(position.scaled_amount),
            market_index.borrow_index,
            feed.price,
        );
        total = total.checked_add(env, value);
    }
    total
}

/// Liquidation-threshold-weighted collateral (WAD) of an arbitrary supply
/// book, valued with `cache`'s prices and supply indexes. Mirrors
/// production's `weighted_collateral` leg of `calculate_account_risk_totals`
/// (floor-rounded value, floor-scaled by the position's frozen threshold).
fn weighted_collateral_wad_of(
    env: &Env,
    cache: &mut crate::context::Cache,
    positions: &Map<HubAssetKey, AccountPositionRaw>,
) -> Wad {
    let keys = positions.keys();
    prime_position_inputs(cache, &keys);
    let mut weighted = Wad::ZERO;
    for hub_asset in keys.iter() {
        let position = positions.get(hub_asset.clone()).unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value_floor(
            env,
            Ray::from(position.scaled_amount),
            market_index.supply_index,
            feed.price,
        );
        weighted = weighted.checked_add(
            env,
            Bps::from(position.liquidation_threshold).apply_to_wad_floor(env, value),
        );
    }
    weighted
}

fn inline_total_borrow_wad(env: &Env, cache: &mut crate::context::Cache, account_id: u64) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    total_borrow_wad_of(env, cache, &account.borrow_positions)
}

fn inline_weighted_collateral_wad(
    env: &Env,
    cache: &mut crate::context::Cache,
    account_id: u64,
) -> Wad {
    let account = crate::storage::get_account(env, account_id);
    weighted_collateral_wad_of(env, cache, &account.supply_positions)
}

/// The post-gate fence, shared by every `post_gate_*` rule.
///
/// Asserts that the position book the post-pool solvency gate valued is worth
/// exactly what the account's *persisted* end-of-transaction book is worth, on
/// both the debt and the weighted-collateral leg. Both snapshots are valued
/// through one `Cache`, so a single frozen price/index basis applies to each
/// side and the comparison isolates the only thing a post-gate step could
/// change: the account's positions and the sides of them that get persisted.
///
/// This is the generalized form of Trail of Bits' TOB-AAVE-7. Their
/// `_refreshAndValidateUserPosition` checked health and `_notifyRiskPremiumUpdate`
/// then added up to 2 wei of debt, so an account admitted at HF = 1 was
/// instantly liquidatable. Any future step inserted after
/// `enforce_post_pool_solvency` / `strategy_finalize`'s gate that touches a
/// value-bearing field — or any regression in the `restamped -> PositionSides`
/// coupling that decides which side gets written — breaks one of these asserts.
///
/// The assertions are implications: the gate returns early for a debt-free
/// account, and `supply`/`repay` carry no post-pool gate at all, so those paths
/// have no observation to contradict.
fn assert_gate_observation_is_final(e: &Env, account_id: u64) {
    let observed = health_ghost::gate_observed();
    let mut cache = crate::context::Cache::new(e);

    let gate_supply = health_ghost::observed_supply(e);
    let gate_debt = health_ghost::observed_debt(e);
    let gate_weighted = weighted_collateral_wad_of(e, &mut cache, &gate_supply);
    let gate_total_debt = total_borrow_wad_of(e, &mut cache, &gate_debt);

    let final_supply = crate::storage::get_supply_positions(e, account_id);
    let final_debt = crate::storage::get_debt_positions(e, account_id);
    let final_weighted = weighted_collateral_wad_of(e, &mut cache, &final_supply);
    let final_total_debt = total_borrow_wad_of(e, &mut cache, &final_debt);

    cvlr_assert!(!observed || final_total_debt.raw() == gate_total_debt.raw());
    cvlr_assert!(!observed || final_weighted.raw() == gate_weighted.raw());
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

fn scaled_supply_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let account = crate::storage::get_account(env, account_id);
    account
        .supply_positions
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

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
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw());

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}

fn nondet_swap_steps() -> crate::types::StrategySwap {
    cvlr_soroban::nondet_bytes1()
}

/// Seeds a single-asset live account and bounds both position books to one
/// entry, the shape every `post_gate_*` verb rule starts from.
fn seed_bounded_account(e: &Env, account_id: u64, caller: &Address, asset: &Address) {
    crate::spec::fixture::seed_live_account(e, account_id, caller, asset);
    let pre_account = crate::storage::get_account(e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);
}

#[rule]
fn post_gate_supply_totals_are_final(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_bounded_account(&e, account_id, &caller, &asset);

    health_ghost::reset();
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_withdraw_totals_are_final(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_bounded_account(&e, account_id, &caller, &asset);

    health_ghost::reset();
    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset, amount);

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_borrow_totals_are_final(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_bounded_account(&e, account_id, &caller, &asset);

    health_ghost::reset();
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_repay_totals_are_final(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    seed_bounded_account(&e, account_id, &caller, &asset);

    health_ghost::reset();
    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_multiply_totals_are_final(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
) {
    let steps = nondet_swap_steps();
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    health_ghost::reset();
    let account_id = crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        crate::spec::fixture::SPOKE_ID,
        collateral_token,
        WAD,
        debt_token,
        1,
        steps,
    );

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_repay_with_collateral_totals_are_final(
    e: Env,
    caller: Address,
    account_id: u64,
    collateral_token: Address,
    collateral_amount: i128,
    debt_token: Address,
    collateral_scaled_before: i128,
    debt_scaled_before: i128,
) {
    let steps = nondet_swap_steps();
    cvlr_assume!(collateral_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    cvlr_assume!(
        collateral_scaled_before > 0 && collateral_scaled_before <= 20 * common::constants::RAY
    );
    cvlr_assume!(debt_scaled_before > 0 && debt_scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_token,
        collateral_scaled_before,
    );
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_token, debt_scaled_before);

    health_ghost::reset();
    crate::spec::compat::repay_debt_with_collateral_minimal(
        e.clone(),
        caller,
        account_id,
        collateral_token,
        collateral_amount,
        debt_token,
        steps,
    );

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_swap_collateral_totals_are_final(
    e: Env,
    caller: Address,
    account_id: u64,
    current_collateral: Address,
    from_amount: i128,
    new_collateral: Address,
    scaled_before: i128,
) {
    let steps = nondet_swap_steps();
    cvlr_assume!(from_amount > 0);
    cvlr_assume!(current_collateral != new_collateral);
    cvlr_assume!(scaled_before > 0 && scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &current_collateral);
    crate::spec::fixture::seed_market(&e, &new_collateral);
    crate::spec::fixture::seed_supply_position(&e, account_id, &current_collateral, scaled_before);

    health_ghost::reset();
    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        hub0(&current_collateral),
        from_amount,
        hub0(&new_collateral),
        steps,
    );

    assert_gate_observation_is_final(&e, account_id);
}

#[rule]
fn post_gate_swap_debt_totals_are_final(
    e: Env,
    caller: Address,
    account_id: u64,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    scaled_before: i128,
) {
    let steps = nondet_swap_steps();
    cvlr_assume!(new_debt_amount > 0);
    cvlr_assume!(existing_debt_token != new_debt_token);
    cvlr_assume!(scaled_before > 0 && scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &existing_debt_token);
    crate::spec::fixture::seed_market(&e, &new_debt_token);
    crate::spec::fixture::seed_debt_position(&e, account_id, &existing_debt_token, scaled_before);

    health_ghost::reset();
    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        hub0(&existing_debt_token),
        new_debt_amount,
        hub0(&new_debt_token),
        steps,
    );

    assert_gate_observation_is_final(&e, account_id);
}

/// Witness: a borrow really does reach the post-pool gate, so
/// `post_gate_borrow_totals_are_final` is not an empty implication.
#[rule]
fn post_gate_borrow_observes_gate_witness(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        WAD * 4,
    );

    health_ghost::reset();
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, WAD);

    cvlr_satisfy!(health_ghost::gate_observed());
}

/// Witness: a withdrawal from an account that carries debt reaches the
/// post-pool gate, so `post_gate_withdraw_totals_are_final` is not an empty
/// implication.
#[rule]
fn post_gate_withdraw_observes_gate_witness(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        WAD * 4,
    );
    crate::spec::compat::borrow_single(e.clone(), caller.clone(), account_id, asset.clone(), WAD);

    health_ghost::reset();
    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset, WAD);

    cvlr_satisfy!(health_ghost::gate_observed());
}

/// Witness: `strategy_finalize`'s gate is reachable, so the strategy-tail
/// post-gate rules are not empty implications.
#[rule]
fn post_gate_multiply_observes_gate_witness(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
) {
    let steps = nondet_swap_steps();
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::fixture::seed_market(&e, &collateral_token);
    crate::spec::fixture::seed_market(&e, &debt_token);

    health_ghost::reset();
    crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        crate::spec::fixture::SPOKE_ID,
        collateral_token,
        WAD,
        debt_token,
        1,
        steps,
    );

    cvlr_satisfy!(health_ghost::gate_observed());
}

/// Witness pinning today's shape: `supply` completes without ever reaching a
/// post-pool solvency gate, which is why
/// `post_gate_supply_totals_are_final` currently holds vacuously.
#[rule]
fn post_gate_supply_skips_gate_witness(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    health_ghost::reset();
    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, WAD);

    cvlr_satisfy!(!health_ghost::gate_observed());
}

/// Witness pinning today's shape: `repay` completes without ever reaching a
/// post-pool solvency gate, which is why
/// `post_gate_repay_totals_are_final` currently holds vacuously.
#[rule]
fn post_gate_repay_skips_gate_witness(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, common::constants::RAY);

    health_ghost::reset();
    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, WAD);

    cvlr_satisfy!(!health_ghost::gate_observed());
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
