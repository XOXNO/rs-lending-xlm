//! Solvency and utilization guards on the SAC-free accounting halves.
//!
//! These guards are invisible to the accounting-domain suites: `fixture::params`
//! sets the `RAY` sentinel that disables the utilization cap, and those rules
//! seed `cash: i128::MAX` so the reserve gate can never bind. Every rule here
//! exists to make one guard actually reachable.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::types::{PoolBorrowEntry, PoolNetSettleEntry, PoolWithdrawEntry};

use super::fixture::{
    action, hub, params, params_with_max_util, position, read_state, seed, state, MAX_FLOW_AMOUNT,
    ONE_TOKEN,
};

/// Utilization after a leg, recomputed from persisted state the way
/// `Cache::calculate_utilization` does.
fn utilization_after(e: &Env, supplied: i128, borrowed: i128, s_idx: i128, b_idx: i128) -> Ray {
    if supplied == 0 {
        return Ray::ZERO;
    }
    let debt_value = Ray::from(borrowed).mul(e, Ray::from(b_idx));
    let supply_value = Ray::from(supplied).mul(e, Ray::from(s_idx));
    common::rates::utilization(e, debt_value, supply_value)
}

/// A borrow can never persist a state above the market's utilization cap.
/// Requires the cap to be genuinely enabled — with the `RAY` sentinel the guard
/// early-returns and this property is vacuous.
#[rule]
fn borrow_respects_utilization_cap(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    position_before: i128,
) {
    let max_util = RAY * 9 / 10;
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(position_before >= 0 && position_before <= borrowed);

    seed(
        &e,
        admin,
        asset.clone(),
        params_with_max_util(asset.clone(), max_util),
        state(
            supplied,
            borrowed,
            0,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolBorrowEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::borrow::accounting(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        utilization_after(
            &e,
            post.supplied,
            post.borrowed,
            post.supply_index,
            post.borrow_index
        )
        .raw()
            <= max_util
    );
}

/// A non-liquidation withdrawal is subject to the same cap. Liquidation
/// seizures are exempt by design (`ops::withdraw::accounting`).
#[rule]
fn user_withdraw_respects_utilization_cap(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    position_before: i128,
) {
    let max_util = RAY * 9 / 10;
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(position_before > 0 && position_before <= supplied);

    seed(
        &e,
        admin,
        asset.clone(),
        params_with_max_util(asset.clone(), max_util),
        state(
            supplied,
            borrowed,
            0,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    crate::ops::withdraw::accounting(&e, false, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        utilization_after(
            &e,
            post.supplied,
            post.borrowed,
            post.supply_index,
            post.borrow_index
        )
        .raw()
            <= max_util
    );
}

/// A withdrawal never pays out more than tracked cash, and never drives cash
/// negative. Exercises `Cache::require_reserves`, which the position rules
/// disable by seeding `cash: i128::MAX`.
#[rule]
fn withdraw_never_overdraws_cash(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    cash_before: i128,
    supply_index: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            0,
            RAY,
            supply_index,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    let outcome = crate::ops::withdraw::accounting(&e, false, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(outcome.net_transfer <= pre.cash);
    cvlr_assert!(pre.cash - post.cash == outcome.net_transfer);
    cvlr_assert!(post.cash >= 0);
}

/// Burning supply shares can never leave protocol revenue exceeding the shares
/// outstanding. Withdraw is one of only two paths that shrink `supplied`.
#[rule]
fn withdraw_keeps_revenue_backed(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    revenue_before: i128,
    protocol_fee: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 100 * RAY);
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= MAX_FLOW_AMOUNT);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            revenue_before,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee,
    };
    crate::ops::withdraw::accounting(&e, true, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
}

/// The other shrinking path: net settlement burns supply and debt together.
#[rule]
fn net_settle_keeps_revenue_backed(
    e: Env,
    admin: Address,
    asset: Address,
    requested: i128,
    supply_before: i128,
    debt_before: i128,
    revenue_before: i128,
) {
    cvlr_assume!(requested >= 0 && requested <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_before > 0 && supply_before <= 20 * RAY);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 20 * RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            50 * RAY,
            revenue_before,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: requested,
        supply_position: position(supply_before),
        debt_position: position(debt_before),
    };
    crate::ops::net_settle::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
}

/// A withdrawal never persists supply drained to zero while debt remains —
/// the same terminal-state guard `net_settle` already proves.
#[rule]
fn withdraw_leaves_no_orphan_debt(
    e: Env,
    admin: Address,
    asset: Address,
    position_before: i128,
    borrowed: i128,
    borrow_index: i128,
) {
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(borrowed > 0 && borrowed <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            position_before,
            borrowed,
            0,
            borrow_index,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    // Full-close sentinel: burn the entire position.
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, i128::MAX),
        protocol_fee: 0,
    };
    crate::ops::withdraw::accounting(&e, true, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(!(post.supplied == 0 && post.borrowed != 0));
}

/// Claiming protocol revenue cannot drain supply to zero under live debt.
#[rule]
fn claim_revenue_leaves_no_orphan_debt(
    e: Env,
    admin: Address,
    asset: Address,
    revenue_before: i128,
    borrowed: i128,
    cash_before: i128,
) {
    cvlr_assume!(revenue_before > 0 && revenue_before <= 20 * RAY);
    cvlr_assume!(borrowed > 0 && borrowed <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        // Every outstanding share is protocol-owned: the worst case for the guard.
        state(
            revenue_before,
            borrowed,
            revenue_before,
            RAY,
            RAY,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    crate::ops::revenue::accounting(&e, hub(asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(!(post.supplied == 0 && post.borrowed != 0));
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.cash >= 0);
}
