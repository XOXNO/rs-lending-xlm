use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    BPS, MAX_ASSET_DECIMALS, MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, MIN_ASSET_DECIMALS, RAY,
    SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::types::{AccountPositionType, PoolNetSettleEntry, PoolSeizeEntry};

use super::fixture::{
    hub, nondet_params, params_with_decimals_and_reserve, position, read_state, seed, state,
    MAX_FLOW_AMOUNT, ONE_TOKEN,
};

// Fixture domain: these rules draw `asset_decimals` and `reserve_factor` over
// the ranges production validates (`MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`,
// `reserve_factor < BPS`) rather than pinning `params`'s mainnet-shaped 7 and
// 1_000. The rate curve stays fixed; see `certora/pool/spec/README.md`.

#[rule]
fn seize_borrow_reduces_debt_and_writes_down_supply(
    e: Env,
    admin: Address,
    asset: Address,
    seized_scaled: i128,
    borrow_index: i128,
    supply_index: i128,
) {
    // `fixture::state` stamps `last_timestamp = e.ledger().timestamp() * 1_000`
    // and `Cache::load` recomputes the same product through `time::now_ms`.
    // Both are checked multiplications, so a ledger clock past `u64::MAX /
    // 1_000` panics and Sunbeam prunes the path as `assume(false)`. Stating the
    // bound makes that pruning visible instead of hidden, and drops the
    // overflow branch from every rule below.
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(seized_scaled >= 0 && seized_scaled <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    let supplied = 100 * RAY;
    seed(
        &e,
        admin,
        asset.clone(),
        nondet_params(asset.clone()),
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
    let bad_debt = Ray::from(seized_scaled).mul_ceil(&e, Ray::from(pre.borrow_index));
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
    crate::ops::seize::apply(&e, &entry);
    let post = read_state(&e, &asset);
    let post_value = Ray::from(post.supplied).mul(&e, Ray::from(post.supply_index));
    let floor_value = Ray::from(post.supplied).mul(&e, Ray::from(SUPPLY_INDEX_FLOOR_RAW));

    cvlr_assert!(pre.borrowed - post.borrowed == seized_scaled);
    cvlr_assert!(post.supply_index == expected_index.raw());
    cvlr_assert!(post.supply_index <= pre.supply_index);
    cvlr_assert!(post.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
    cvlr_assert!(post.cash == pre.cash && post.borrow_index == pre.borrow_index);
    cvlr_assert!(
        proportional.raw() < SUPPLY_INDEX_FLOOR_RAW || post_value.raw() <= remaining.raw()
    );
    cvlr_assert!(
        proportional.raw() >= SUPPLY_INDEX_FLOOR_RAW
            || (post.supply_index == SUPPLY_INDEX_FLOOR_RAW && post_value == floor_value)
    );
}

#[rule]
fn seize_deposit_moves_scaled_position_to_revenue(
    e: Env,
    admin: Address,
    asset: Address,
    seized_scaled: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(seized_scaled >= 0 && seized_scaled <= 20 * RAY);
    seed(
        &e,
        admin,
        asset.clone(),
        nondet_params(asset.clone()),
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
    crate::ops::seize::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue - pre.revenue == seized_scaled);
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.supplied == pre.supplied && post.borrowed == pre.borrowed);
    cvlr_assert!(post.cash == pre.cash);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}

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
    asset_decimals: u32,
    reserve_factor: u32,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(requested >= 0 && requested <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_before >= 0 && supply_before <= 20 * RAY);
    cvlr_assume!(debt_before >= 0 && debt_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    // Production range, not `RAY_DECIMALS`: governance validates
    // `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` and `MarketParamsRaw::verify`
    // caps at `WAD_DECIMALS`, so 0..=2 and 19..=27 are unreachable markets.
    cvlr_assume!((MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&asset_decimals));
    cvlr_assume!(i128::from(reserve_factor) < BPS);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals_and_reserve(asset.clone(), asset_decimals, reserve_factor),
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
    let supply_position = Ray::from(supply_before);
    let debt_position = Ray::from(debt_before);
    let supply_index_ray = Ray::from(supply_index);
    let borrow_index_ray = Ray::from(borrow_index);

    let (expected_supply_burn, expected_debt_burn, expected_gross) =
        common::rates::resolve_net_settle(
            &e,
            requested,
            supply_position,
            debt_position,
            supply_index_ray,
            borrow_index_ray,
            asset_decimals,
        );
    let debt_due = debt_position
        .mul_ceil(&e, borrow_index_ray)
        .to_asset_ceil(&e, asset_decimals);
    let supply_floor = supply_position
        .mul_floor(&e, supply_index_ray)
        .to_asset_floor(&e, asset_decimals);
    let capped = requested.min(debt_due).min(supply_floor);
    cvlr_assume!(
        expected_gross == 0 || (expected_supply_burn.raw() > 0 && expected_debt_burn.raw() > 0)
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: requested,
        supply_position: position(supply_before),
        debt_position: position(debt_before),
    };
    let (result, _) = crate::ops::net_settle::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(expected_gross <= capped && capped <= requested);
    cvlr_assert!(expected_gross <= debt_due && expected_gross <= supply_floor);
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
    cvlr_assert!(
        result.settled_amount == 0
            || (expected_supply_burn.raw() > 0 && expected_debt_burn.raw() > 0)
    );
}

#[rule]
fn net_settle_never_persists_supply_drained_with_debt(
    e: Env,
    admin: Address,
    asset: Address,
    requested: i128,
    supply_scaled: i128,
    extra_debt: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= 20 * RAY);
    cvlr_assume!(extra_debt > 0 && extra_debt <= 20 * RAY);
    cvlr_assume!(requested >= 0 && requested <= MAX_FLOW_AMOUNT);

    let debt_scaled = supply_scaled + extra_debt;
    seed(
        &e,
        admin,
        asset.clone(),
        nondet_params(asset.clone()),
        state(
            supply_scaled,
            debt_scaled,
            0,
            RAY,
            RAY,
            50 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: requested,
        supply_position: position(supply_scaled),
        debt_position: position(debt_scaled),
    };
    let (_result, _) = crate::ops::net_settle::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(!(post.supplied == 0 && post.borrowed != 0));
}

#[rule]
fn bad_debt_writedown_is_noop_on_empty_market(
    e: Env,
    admin: Address,
    asset: Address,
    seized_scaled: i128,
    borrowed: i128,
    supply_index: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(borrowed > 0 && borrowed <= 20 * RAY);
    cvlr_assume!(seized_scaled >= 0 && seized_scaled <= borrowed);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        nondet_params(asset.clone()),
        state(
            0,
            borrowed,
            0,
            RAY,
            supply_index,
            80 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(seized_scaled),
    };
    crate::ops::seize::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.supply_index == pre.supply_index);
    cvlr_assert!(pre.borrowed - post.borrowed == seized_scaled);
    cvlr_assert!(post.cash == pre.cash && post.supplied == 0);
}
