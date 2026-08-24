use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{vec, Address, Env};

use common::constants::{
    MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, RAY_DECIMALS, SUPPLY_INDEX_FLOOR_RAW,
};

use super::fixture::{hub, params, params_with_decimals, read_state, seed, state, ONE_TOKEN};

#[rule]
fn market_create_writes_zeroed_state(e: Env, asset: Address, asset_decimals: u32) {
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    crate::ops::market::create(
        &e,
        0,
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
    );
    let post = read_state(&e, &asset);

    cvlr_assert!(post.supplied == 0 && post.borrowed == 0 && post.revenue == 0);
    cvlr_assert!(post.cash == 0);
    cvlr_assert!(post.borrow_index == RAY && post.supply_index == RAY);
    cvlr_assert!(post.last_timestamp == crate::time::now_ms(&e));
}

/// Certora Hub M-03 analogue: creating a market on a `HubAssetKey` that already
/// exists must revert, never silently re-zero live accounting.
///
/// Counterpart of [`market_create_writes_zeroed_state`]: that rule pins *what*
/// a first create writes; this one pins that a second create on the same key
/// can never write it again. Aave's `addSpoke()` re-call zeroed the spoke's
/// accounting and broke solvency; our guard is `ops/market.rs:24`
/// (`assert_with_error!(!market_exists, AssetAlreadySupported)`).
///
/// The market is pre-seeded with *live* accounting (non-zero supply and debt),
/// which is the state Aave's bug destroyed.
///
/// Revert shape: the trailing assert is reachable only if `create` returns, so
/// the rule verifies exactly when every path panics. The params passed here are
/// an instance (`asset_decimals == 7`) of the params domain
/// `market_create_writes_zeroed_state` already drives `create` through, so
/// `params.verify` is known to accept them — the duplicate-key guard is
/// therefore the only remaining revert source, and the rule cannot pass for the
/// wrong reason.
#[rule]
fn market_duplicate_create_reverts(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
) {
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
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

    crate::ops::market::create(&e, 0, params(asset.clone(), 0, false));

    cvlr_assert!(false);
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn accrue_is_noop_when_no_time_elapsed(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(cash >= 0 && cash <= 1_000 * ONE_TOKEN);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
            cash,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    crate::ops::market::accrue(&e, vec![&e, hub(asset.clone())]);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.supplied == pre.supplied && post.borrowed == pre.borrowed);
    cvlr_assert!(post.revenue == pre.revenue && post.cash == pre.cash);
    cvlr_assert!(post.supply_index == pre.supply_index);
    cvlr_assert!(post.borrow_index == pre.borrow_index);
    cvlr_assert!(post.last_timestamp == pre.last_timestamp);
}
