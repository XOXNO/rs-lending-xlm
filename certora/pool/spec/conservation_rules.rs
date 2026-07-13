//! Δ-conservation rules: every pool operation moves the market aggregate by
//! exactly the delta of the returned position, and moves `cash` by exactly
//! the token amount. Together with the controller-side persistence proofs
//! (consistency_rules) these give Σ(account scaled) == market total by
//! induction, without ghost sums.
//!
//! Self-contained setup mirrors integrity_rules.rs (kept private there);
//! staying additive avoids churn in the actively-developed integrity file.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::types::{
    HubAssetKey, MarketParamsRaw, PoolAction, PoolKey, PoolStateRaw, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;

const MAX_AMOUNT: i128 = 1_000_000_000_000i128;

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

fn valid_params(asset: Address) -> MarketParamsRaw {
    MarketParamsRaw {
        base_borrow_rate: RAY / 100,
        slope1: RAY / 10,
        slope2: RAY / 5,
        slope3: RAY / 2,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_borrow_rate: 2 * RAY,
        max_utilization: RAY,
        reserve_factor: 1_000,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: 7,
    }
}

fn valid_state(supplied: i128, borrowed: i128, revenue: i128, timestamp: u64) -> PoolStateRaw {
    PoolStateRaw {
        supplied,
        borrowed,
        revenue,
        borrow_index: RAY,
        supply_index: RAY,
        last_timestamp: timestamp * 1000,
        cash: supplied.saturating_sub(borrowed),
    }
}

fn seed_pool(env: &Env, admin: Address, asset: Address, state: PoolStateRaw) {
    crate::LiquidityPool::__constructor(env.clone(), admin);
    env.storage().persistent().set(
        &PoolKey::Params(hub(asset.clone())),
        &valid_params(asset.clone()),
    );
    env.storage()
        .persistent()
        .set(&PoolKey::State(hub(asset)), &state);
}

fn read_state(env: &Env, asset: &Address) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub(asset.clone())))
        .unwrap()
}

fn position(scaled_amount: i128) -> ScaledPositionRaw {
    ScaledPositionRaw { scaled_amount }
}

fn action(position: ScaledPositionRaw, amount: i128, asset: Address) -> PoolAction {
    PoolAction {
        position,
        amount,
        hub_asset: hub(asset),
    }
}

fn supply_first(e: &Env, act: PoolAction) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolSupplyEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolSupplyEntry { action: act });
    crate::LiquidityPool::supply(e.clone(), entries).get_unchecked(0)
}

fn borrow_first(
    e: &Env,
    receiver: Address,
    act: PoolAction,
) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolBorrowEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolBorrowEntry { action: act });
    crate::LiquidityPool::borrow(e.clone(), receiver, entries).get_unchecked(0)
}

fn withdraw_first(
    e: &Env,
    receiver: Address,
    act: PoolAction,
    is_liquidation: bool,
    protocol_fee: i128,
) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolWithdrawEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolWithdrawEntry {
        action: act,
        protocol_fee,
    });
    crate::LiquidityPool::withdraw(e.clone(), receiver, is_liquidation, entries).get_unchecked(0)
}

fn repay_first(e: &Env, payer: Address, act: PoolAction) -> common::types::PoolPositionMutation {
    let mut actions: soroban_sdk::Vec<PoolAction> = soroban_sdk::Vec::new(e);
    actions.push_back(act);
    crate::LiquidityPool::repay(e.clone(), payer, actions).get_unchecked(0)
}

/// supply: supplied delta == returned position delta; cash delta == amount.
#[rule]
fn supply_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((0..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = supply_first(&e, action(before.clone(), amount, asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(
        post.supplied - pre.supplied == result.position.scaled_amount - before.scaled_amount
    );
    cvlr_assert!(post.borrowed == pre.borrowed);
    cvlr_assert!(post.cash - pre.cash == result.actual_amount);
}

/// withdraw: supplied delta == position delta; cash decreases by the full
/// actual amount when no protocol fee is retained.
#[rule]
fn withdraw_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin.clone(),
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = withdraw_first(
        &e,
        admin,
        action(before.clone(), amount, asset.clone()),
        false,
        0,
    );
    let post = read_state(&e, &asset);

    cvlr_assert!(
        pre.supplied - post.supplied == before.scaled_amount - result.position.scaled_amount
    );
    cvlr_assert!(post.borrowed == pre.borrowed);
    // protocol_fee = 0 in this rule, so the whole actual amount leaves as cash.
    cvlr_assert!(pre.cash - post.cash == result.actual_amount);
}

/// borrow: borrowed delta == position delta; cash decreases by amount.
#[rule]
fn borrow_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    receiver: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((0..=50 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = borrow_first(&e, receiver, action(before.clone(), amount, asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(
        post.borrowed - pre.borrowed == result.position.scaled_amount - before.scaled_amount
    );
    cvlr_assert!(post.supplied == pre.supplied);
    cvlr_assert!(pre.cash - post.cash == result.actual_amount);
}

/// repay: borrowed delta == position delta; cash grows by the amount
/// actually applied (over-payment is refunded, not retained).
#[rule]
fn repay_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    payer: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = repay_first(&e, payer, action(before.clone(), amount, asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(
        pre.borrowed - post.borrowed == before.scaled_amount - result.position.scaled_amount
    );
    cvlr_assert!(post.supplied == pre.supplied);
    cvlr_assert!(post.cash - pre.cash == result.actual_amount);
}

/// Bulk supply with two entries on the same asset conserves the aggregate:
/// the market total moves by the sum of both position deltas (covers the
/// bulk-loop accumulation, not just the bulk-of-one body).
#[rule]
fn supply_bulk_two_entries_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount1: i128,
    amount2: i128,
) {
    cvlr_assume!(amount1 > 0 && amount1 <= MAX_AMOUNT);
    cvlr_assume!(amount2 > 0 && amount2 <= MAX_AMOUNT);
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let mut entries: soroban_sdk::Vec<common::types::PoolSupplyEntry> = soroban_sdk::Vec::new(&e);
    entries.push_back(common::types::PoolSupplyEntry {
        action: action(position(0), amount1, asset.clone()),
    });
    entries.push_back(common::types::PoolSupplyEntry {
        action: action(position(0), amount2, asset.clone()),
    });
    let results = crate::LiquidityPool::supply(e.clone(), entries);
    let post = read_state(&e, &asset);

    let delta = results.get_unchecked(0).position.scaled_amount
        + results.get_unchecked(1).position.scaled_amount;
    cvlr_assert!(post.supplied - pre.supplied == delta);
}

/// Re-registering an existing market must revert (would otherwise zero the
/// live aggregates).
#[rule]
fn create_market_rejects_existing(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 25 * RAY, RAY, e.ledger().timestamp()),
    );

    crate::LiquidityPool::create_market(e.clone(), 0, valid_params(asset.clone()));

    // Assert-unreachable form: Verified iff the second registration traps on
    // every path. (satisfy-form rules on this pool WASM die in the OSS
    // prover's presolver — instant "Violated (unsat)" after a
    // ConstArrayInitSummary warning — while the assert pipeline works.)
    cvlr_assert!(false);
}

#[rule]
fn pool_conservation_reachability(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );
    let result = supply_first(&e, action(position(0), amount, asset.clone()));
    cvlr_satisfy!(result.position.scaled_amount > 0);
}
