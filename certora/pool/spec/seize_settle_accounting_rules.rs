//! Bad-debt seizure, deposit seizure, and zero-cash net settlement.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::types::{AccountPositionType, PoolNetSettleEntry, PoolSeizeEntry};

use super::fixture::{hub, params, position, read_state, seed, state, MAX_FLOW_AMOUNT, ONE_TOKEN};

/// Borrow seizure removes the exact debt shares and applies the production
/// proportional write-down, saturated only by the supply-index floor.
#[rule]
fn seize_borrow_reduces_debt_and_writes_down_supply(
    e: Env,
    admin: Address,
    asset: Address,
    seized_scaled: i128,
    borrow_index: i128,
    supply_index: i128,
) {
    cvlr_assume!(seized_scaled >= 0 && seized_scaled <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= 10 * RAY);
    let supplied = 100 * RAY;
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            supplied,
            50 * RAY,
            5 * RAY,
            borrow_index,
            supply_index,
            50 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let total_value = Ray::from(pre.supplied).mul(&e, Ray::from(pre.supply_index));
    let bad_debt = Ray::from(seized_scaled).mul(&e, Ray::from(pre.borrow_index));
    let capped = bad_debt.min(total_value);
    let remaining = total_value.checked_sub(&e, capped);
    let factor = remaining.div_floor(&e, total_value);
    let proportional = Ray::from(pre.supply_index).mul_floor(&e, factor);
    let expected_index = proportional.max(Ray::from(SUPPLY_INDEX_FLOOR_RAW));

    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(seized_scaled),
    };
    crate::seize_one(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(pre.borrowed - post.borrowed == seized_scaled);
    cvlr_assert!(post.supply_index == expected_index.raw());
    cvlr_assert!(post.supply_index <= pre.supply_index);
    cvlr_assert!(post.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
    cvlr_assert!(post.cash == pre.cash && post.borrow_index == pre.borrow_index);
}

/// Seizing an already-aggregated deposit transfers its shares to protocol
/// revenue; aggregate supply itself must not change.
#[rule]
fn seize_deposit_moves_scaled_position_to_revenue(
    e: Env,
    admin: Address,
    asset: Address,
    seized_scaled: i128,
) {
    cvlr_assume!(seized_scaled >= 0 && seized_scaled <= 20 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            5 * RAY,
            RAY,
            RAY,
            80 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Deposit,
        position: position(seized_scaled),
    };
    crate::seize_one(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue - pre.revenue == seized_scaled);
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.supplied == pre.supplied && post.borrowed == pre.borrowed);
    cvlr_assert!(post.cash == pre.cash);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}

/// Net settlement uses one common gross amount for both legs, changes both
/// aggregates by their returned position deltas, and never moves cash.
#[rule]
#[allow(clippy::too_many_arguments)]
fn net_settle_conserves_cash_and_both_scaled_totals(
    e: Env,
    admin: Address,
    asset: Address,
    requested: i128,
    supply_before: i128,
    debt_before: i128,
    supply_index: i128,
    borrow_index: i128,
) {
    cvlr_assume!(requested >= 0 && requested <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_before >= 0 && supply_before <= 20 * RAY);
    cvlr_assume!(debt_before >= 0 && debt_before <= 20 * RAY);
    cvlr_assume!(supply_index >= RAY && supply_index <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= 10 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            50 * RAY,
            5 * RAY,
            borrow_index,
            supply_index,
            50 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    let supply_position = Ray::from(supply_before);
    let debt_position = Ray::from(debt_before);
    let capped = requested.min(cache.unscale_borrow_ceil(debt_position));
    let (expected_supply_burn, expected_gross) = cache.resolve_withdrawal(capped, supply_position);
    let (expected_debt_burn, expected_overpayment) =
        cache.resolve_repay(expected_gross, debt_position);

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: requested,
        supply_position: position(supply_before),
        debt_position: position(debt_before),
    };
    let (result, _) = crate::net_settle_one(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(expected_overpayment == 0);
    cvlr_assert!(result.settled_amount == expected_gross);
    cvlr_assert!(
        supply_before - result.supply_position.scaled_amount == expected_supply_burn.raw()
    );
    cvlr_assert!(debt_before - result.debt_position.scaled_amount == expected_debt_burn.raw());
    cvlr_assert!(pre.supplied - post.supplied == expected_supply_burn.raw());
    cvlr_assert!(pre.borrowed - post.borrowed == expected_debt_burn.raw());
    cvlr_assert!(post.cash == pre.cash && post.revenue == pre.revenue);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
    cvlr_assert!(result.settled_amount >= 0 && result.settled_amount <= requested);
}
