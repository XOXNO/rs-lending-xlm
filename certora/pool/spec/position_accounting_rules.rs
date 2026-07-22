//! Exact single-leg supply, borrow, withdraw, and repay accounting.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, RAY_DECIMALS, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::math::fp_core;
use common::types::{PoolBorrowEntry, PoolSupplyEntry, PoolWithdrawEntry};

use super::fixture::{
    action, params_with_decimals, read_state, seed, state, MAX_FLOW_AMOUNT, ONE_TOKEN,
};

/// Supply mints the index-scaled shares to both the account result and aggregate.
#[rule]
fn supply_scaled_balance_matches_index(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= 10 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected = fp_core::mul_div_floor(&e, amount_ray.raw(), RAY, supply_index);
    cvlr_assume!(expected > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            100 * RAY,
            RAY,
            supply_index.max(RAY),
            supply_index,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolSupplyEntry {
        action: action(asset.clone(), position_before, amount),
    };
    let (result, _) = crate::supply_one(&e, &entry);
    let post = read_state(&e, &asset);
    let pre_claim = Ray::from(pre.supplied)
        .mul_floor(&e, Ray::from(pre.supply_index))
        .to_asset_floor(asset_decimals);
    let pre_debt = Ray::from(pre.borrowed)
        .mul_ceil(&e, Ray::from(pre.borrow_index))
        .to_asset_ceil(asset_decimals);
    let post_claim = Ray::from(post.supplied)
        .mul_floor(&e, Ray::from(post.supply_index))
        .to_asset_floor(asset_decimals);
    let post_debt = Ray::from(post.borrowed)
        .mul_ceil(&e, Ray::from(post.borrow_index))
        .to_asset_ceil(asset_decimals);
    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount - position_before == expected);
    cvlr_assert!(Ray::from(expected).mul_floor(&e, Ray::from(supply_index)) <= amount_ray);
    cvlr_assert!(post.supplied - pre.supplied == expected);
    cvlr_assert!(post.cash - pre.cash == amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.revenue == pre.revenue);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
    cvlr_assert!(pre_claim <= pre.cash.saturating_add(pre_debt));
    cvlr_assert!(post_claim <= post.cash.saturating_add(post_debt));
}

/// Borrow mints index-scaled debt and debits exactly the borrowed cash amount.
#[rule]
fn borrow_scaled_debt_matches_index(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before >= 0 && debt_before <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected = fp_core::mul_div_ceil(&e, amount_ray.raw(), RAY, borrow_index);
    cvlr_assume!(expected > 0 && expected <= i128::MAX - debt_before);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolBorrowEntry {
        action: action(asset.clone(), debt_before, amount),
    };
    let (_, result, _) = crate::borrow_accounting(&e, &entry);
    let post = read_state(&e, &asset);
    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount - debt_before == expected);
    cvlr_assert!(Ray::from(expected).mul_floor(&e, Ray::from(borrow_index)) >= amount_ray);
    cvlr_assert!(post.borrowed - pre.borrowed == expected);
    cvlr_assert!(pre.cash - post.cash == amount);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}

/// Partial withdrawal burns the index-scaled amount and transfers the gross amount.
#[rule]
fn partial_withdraw_burns_scaled_supply(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let current_actual = Ray::from(position_before)
        .mul(&e, Ray::from(supply_index))
        .to_asset(asset_decimals);
    cvlr_assume!(amount < current_actual);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected_burn = fp_core::mul_div_ceil(&e, amount_ray.raw(), RAY, supply_index);
    cvlr_assume!(expected_burn > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            0,
            RAY,
            RAY,
            supply_index,
            i128::MAX,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    let (_, result, _, net) = crate::withdraw_accounting(&e, false, &entry);
    let post = read_state(&e, &asset);
    cvlr_assert!(result.actual_amount == amount && net == amount);
    cvlr_assert!(position_before - result.position.scaled_amount == expected_burn);
    cvlr_assert!(Ray::from(expected_burn).mul_floor(&e, Ray::from(supply_index)) >= amount_ray);
    cvlr_assert!(pre.supplied - post.supplied == expected_burn);
    cvlr_assert!(pre.cash - post.cash == amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.revenue == pre.revenue);
}

/// The full-withdraw sentinel burns every share and pays the conservative floor value.
#[rule]
fn full_withdraw_burns_entire_position(
    e: Env,
    admin: Address,
    asset: Address,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            0,
            RAY,
            RAY,
            supply_index,
            i128::MAX,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, i128::MAX),
        protocol_fee: 0,
    };
    let (_, result, _, net) = crate::withdraw_accounting(&e, false, &entry);
    let post = read_state(&e, &asset);
    let expected_gross = Ray::from(position_before)
        .mul_floor(&e, Ray::from(supply_index))
        .to_asset_floor(asset_decimals);

    cvlr_assert!(result.position.scaled_amount == 0);
    cvlr_assert!(pre.supplied - post.supplied == position_before);
    cvlr_assert!(result.actual_amount == expected_gross && net == expected_gross);
    cvlr_assert!(pre.cash - post.cash == expected_gross);
}

/// Partial repay burns the borrow-index-scaled amount from debt and aggregate.
#[rule]
fn partial_repay_burns_scaled_debt(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let debt_ceil = Ray::from(debt_before)
        .mul_ceil(&e, Ray::from(borrow_index))
        .to_asset_ceil(asset_decimals);
    cvlr_assume!(amount < debt_ceil);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected_burn = fp_core::mul_div_floor(&e, amount_ray.raw(), RAY, borrow_index);
    cvlr_assume!(expected_burn > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            100 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let act = action(asset.clone(), debt_before, amount);
    let (_, result, _, overpayment) = crate::repay_accounting(&e, &act);
    let post = read_state(&e, &asset);
    cvlr_assert!(overpayment == 0 && result.actual_amount == amount);
    cvlr_assert!(debt_before - result.position.scaled_amount == expected_burn);
    cvlr_assert!(Ray::from(expected_burn).mul_ceil(&e, Ray::from(borrow_index)) <= amount_ray);
    cvlr_assert!(pre.borrowed - post.borrowed == expected_burn);
    cvlr_assert!(post.cash - pre.cash == amount);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
}

/// Full repay burns all debt, credits only debt due, and identifies the refund exactly.
#[rule]
fn full_repay_refunds_overpayment(
    e: Env,
    admin: Address,
    asset: Address,
    debt_before: i128,
    borrow_index: i128,
    extra: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(extra >= 0 && extra <= MAX_FLOW_AMOUNT);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let debt_ceil = Ray::from(debt_before)
        .mul_ceil(&e, Ray::from(borrow_index))
        .to_asset_ceil(asset_decimals);
    let amount = debt_ceil + extra;
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            0,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let act = action(asset.clone(), debt_before, amount);
    let (_, result, _, overpayment) = crate::repay_accounting(&e, &act);
    let post = read_state(&e, &asset);

    cvlr_assert!(result.position.scaled_amount == 0);
    cvlr_assert!(pre.borrowed - post.borrowed == debt_before);
    cvlr_assert!(result.actual_amount == debt_ceil);
    cvlr_assert!(post.cash - pre.cash == debt_ceil);
    cvlr_assert!(overpayment == extra);
}
