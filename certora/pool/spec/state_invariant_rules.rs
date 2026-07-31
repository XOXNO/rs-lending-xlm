use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, MILLISECONDS_PER_YEAR, RAY, RAY_DECIMALS,
    SUPPLY_INDEX_FLOOR_RAW,
};
use common::types::{
    AccountPositionType, PoolBorrowEntry, PoolNetSettleEntry, PoolSeizeEntry, PoolStateRaw,
    PoolSupplyEntry, PoolWithdrawEntry,
};

use super::fixture::{
    action, hub, params, params_with_decimals, position, read_state, seed, state, MAX_FLOW_AMOUNT,
    ONE_TOKEN,
};

const MAX_SHARES: i128 = 100 * RAY;

const MAX_CASH: i128 = 1_000 * ONE_TOKEN;

fn assume_invariant(
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(supplied >= 0 && supplied <= MAX_SHARES);
    cvlr_assume!(borrowed >= 0 && borrowed <= MAX_SHARES);
    cvlr_assume!(revenue >= 0 && revenue <= supplied);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(cash >= 0 && cash <= MAX_CASH);
}

fn assert_invariant(post: &PoolStateRaw) {
    cvlr_assert!(post.supplied >= 0);
    cvlr_assert!(post.borrowed >= 0);
    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assert!(post.supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assert!(post.borrow_index >= RAY);
    cvlr_assert!(post.borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assert!(post.cash >= 0);
}

#[allow(clippy::too_many_arguments)]
fn seed_invariant_market(
    e: &Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    assume_invariant(
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    seed(
        e,
        admin,
        asset.clone(),
        params(asset, 0, false),
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
}

#[rule]
fn invariant_holds_after_market_create(e: Env, asset: Address, asset_decimals: u32) {
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    crate::ops::market::create(
        &e,
        0,
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
    );

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_supply(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSupplyEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::supply::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_borrow(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolBorrowEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::borrow::accounting(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_withdraw(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    protocol_fee: i128,
    position_before: i128,
    is_liquidation: bool,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee,
    };
    crate::ops::withdraw::accounting(&e, is_liquidation, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_repay(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let repay_action = action(asset.clone(), position_before, amount);
    crate::ops::repay::accounting(&e, &repay_action);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_net_settle(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supply_position: i128,
    debt_position: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_position >= 0 && supply_position <= MAX_SHARES);
    cvlr_assume!(debt_position >= 0 && debt_position <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount,
        supply_position: position(supply_position),
        debt_position: position(debt_position),
    };
    crate::ops::net_settle::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_seize_borrow(
    e: Env,
    admin: Address,
    asset: Address,
    seized: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(seized >= 0 && seized <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Borrow,
        position: position(seized),
    };
    crate::ops::seize::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_seize_deposit(
    e: Env,
    admin: Address,
    asset: Address,
    seized: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(seized >= 0 && seized <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let entry = PoolSeizeEntry {
        hub_asset: hub(asset.clone()),
        side: AccountPositionType::Deposit,
        position: position(seized),
    };
    crate::ops::seize::apply(&e, &entry);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_add_rewards(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::rewards::apply(&e, hub(asset.clone()), amount);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_claim_revenue(
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
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::revenue::accounting(&e, hub(asset.clone()));

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_recapitalize(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    crate::ops::recapitalize::accounting(&e, hub(asset.clone()), amount);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_create_strategy(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    charge_fee: bool,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= MAX_SHARES);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let strategy_action = action(asset.clone(), position_before, amount);
    crate::ops::strategy::accounting(&e, strategy_action, charge_fee);

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_flash_fee_booking(
    e: Env,
    admin: Address,
    asset: Address,
    fee: i128,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    cvlr_assume!(fee >= 0 && fee <= MAX_FLOW_AMOUNT);
    seed_invariant_market(
        &e,
        admin,
        asset.clone(),
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );

    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::ops::flash::book_fee(&mut cache, fee);
    cache.commit();

    assert_invariant(&read_state(&e, &asset));
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn invariant_preserved_by_one_chunk_accrual(
    e: Env,
    admin: Address,
    asset: Address,
    delta_ms: u64,
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
) {
    assume_invariant(
        supplied,
        borrowed,
        revenue,
        supply_index,
        borrow_index,
        cash,
    );
    cvlr_assume!(delta_ms > 0 && delta_ms <= MILLISECONDS_PER_YEAR);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let current_timestamp = crate::time::now_ms(&e);
    cvlr_assume!(delta_ms <= current_timestamp);

    let mut initial_state = state(
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        cash,
        e.ledger().timestamp(),
    );
    initial_state.last_timestamp = current_timestamp - delta_ms;
    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        initial_state,
    );

    crate::ops::market::accrue(&e, hub(asset.clone()));

    assert_invariant(&read_state(&e, &asset));
}
