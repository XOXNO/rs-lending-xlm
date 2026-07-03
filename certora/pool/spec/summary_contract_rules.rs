use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPositionType, HubAssetKey, MarketParamsRaw, PoolAction, PoolKey, PoolStateRaw,
    ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub(asset: Address) -> HubAssetKey {
    HubAssetKey { hub_id: 0, asset }
}

fn params(asset: Address) -> MarketParamsRaw {
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

fn state(supplied: i128, borrowed: i128, revenue: i128, timestamp: u64) -> PoolStateRaw {
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

fn seed(env: &Env, admin: Address, asset: Address, state: PoolStateRaw) {
    crate::LiquidityPool::__constructor(env.clone(), admin);
    env.storage()
        .persistent()
        .set(&PoolKey::Params(hub(asset.clone())), &params(asset.clone()));
    env.storage()
        .persistent()
        .set(&PoolKey::State(hub(asset)), &state);
}

fn position(scaled: i128) -> ScaledPositionRaw {
    ScaledPositionRaw {
        scaled_amount: scaled,
    }
}

fn action(position: ScaledPositionRaw, amount: i128, asset: Address) -> PoolAction {
    PoolAction {
        position,
        amount,
        hub_asset: hub(asset),
    }
}

// Bulk-of-one wrappers: one entry through the bulk endpoint.
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

fn seize_first(e: &Env, side: AccountPositionType, asset: Address, pos: ScaledPositionRaw) {
    let mut entries: soroban_sdk::Vec<common::types::PoolSeizeEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolSeizeEntry {
        hub_asset: hub(asset),
        side,
        position: pos,
    });
    crate::LiquidityPool::seize_positions(e.clone(), entries);
}

fn read_state(env: &Env, asset: &Address) -> PoolStateRaw {
    env.storage()
        .persistent()
        .get(&PoolKey::State(hub(asset.clone())))
        .unwrap()
}

#[rule]
fn supply_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(RAY);
    let result = supply_first(&e, action(before.clone(), amount, asset));

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount >= before.scaled_amount);
    cvlr_assert!(result.market_index.borrow_index >= RAY);
    cvlr_assert!(result.market_index.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn borrow_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = borrow_first(&e, caller, action(before.clone(), amount, asset));

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount >= before.scaled_amount);
    cvlr_assert!(result.market_index.borrow_index >= RAY);
    cvlr_assert!(result.market_index.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn withdraw_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = withdraw_first(&e, caller, action(before.clone(), amount, asset), false, 0);

    // `resolve_withdrawal` returns `current_supply_floor <= current_supply_actual
    // <= amount` on a full close and `amount` on a partial, so `actual <= amount`
    // holds unconditionally — matching the summary's bound.
    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount <= before.scaled_amount);
    cvlr_assert!(result.position.scaled_amount >= 0);
}

#[rule]
fn repay_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = repay_first(&e, caller, action(before.clone(), amount, asset));

    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount <= before.scaled_amount);
    cvlr_assert!(result.position.scaled_amount >= 0);
}

#[rule]
fn create_strategy_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    fee: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!(fee >= 0 && fee <= amount);
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = crate::LiquidityPool::create_strategy(
        e,
        caller,
        action(before.clone(), amount, asset),
        fee,
    );

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.amount_received == amount - fee);
    cvlr_assert!(result.position.scaled_amount >= before.scaled_amount);
    cvlr_assert!(result.market_index.borrow_index >= RAY);
    cvlr_assert!(result.market_index.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn seize_position_satisfies_controller_summary_contract(
    e: Env,
    admin: Address,
    asset: Address,
    scaled: i128,
) {
    cvlr_assume!((1..=100 * RAY).contains(&scaled));
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    seize_first(
        &e,
        AccountPositionType::Borrow,
        asset.clone(),
        position(scaled),
    );
    // The summary models seize as a no-return state mutation: the seized debt
    // shares leave the market and the indexes stay inside the nondet bounds.
    let after = read_state(&e, &asset);
    cvlr_assert!(after.borrowed == 0);
    cvlr_assert!(after.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assert!(after.borrow_index >= RAY);
}

#[rule]
fn claim_revenue_satisfies_controller_summary_contract(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset.clone(),
        state(100 * RAY, 0, RAY, e.ledger().timestamp()),
    );

    // Claimed amount is non-negative and never exceeds pre-call reserves: the
    // solvency check gates the transfer at `cash`, and `get_reserves() == cash`.
    let pre_reserves = crate::LiquidityPool::get_reserves(e.clone(), hub(asset.clone()));
    let amount = crate::LiquidityPool::claim_revenue(e, hub(asset)).actual_amount;
    cvlr_assert!(amount >= 0);
    cvlr_assert!(amount <= pre_reserves);
}

#[rule]
fn flash_loan_satisfies_fee_domain(
    e: Env,
    admin: Address,
    asset: Address,
    receiver: Address,
    amount: i128,
    fee: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!(fee >= 0 && fee <= amount);
    seed(
        &e,
        admin.clone(),
        asset.clone(),
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let revenue_before = crate::LiquidityPool::get_revenue(e.clone(), hub(asset.clone()));
    crate::LiquidityPool::flash_loan(
        e.clone(),
        hub(asset.clone()),
        admin,
        receiver,
        amount,
        fee,
        Bytes::new(&e),
    );
    let revenue_after = crate::LiquidityPool::get_revenue(e, hub(asset));

    cvlr_assert!(revenue_after == revenue_before + fee);
    cvlr_satisfy!(true);
}

// View bounds on real LiquidityPool views over seeded valid state.

#[allow(clippy::too_many_arguments)]
fn view_state(
    supplied: i128,
    borrowed: i128,
    revenue: i128,
    supply_index: i128,
    borrow_index: i128,
    cash: i128,
    timestamp: u64,
) -> PoolStateRaw {
    PoolStateRaw {
        supplied,
        borrowed,
        revenue,
        borrow_index,
        supply_index,
        last_timestamp: timestamp * 1000,
        cash,
    }
}

/// `get_reserves` is non-negative when `cash >= 0`.
#[rule]
fn reserves_view_nonneg(e: Env, admin: Address, asset: Address, cash: i128) {
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&cash));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(10 * RAY, 0, 0, RAY, RAY, cash, e.ledger().timestamp()),
    );
    cvlr_assert!(crate::LiquidityPool::get_reserves(e, hub(asset)) >= 0);
}

/// `get_supplied_amount` is non-negative under valid state.
#[rule]
fn supplied_amount_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    supply_index: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=10 * RAY).contains(&supply_index));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            0,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::get_supplied_amount(e, hub(asset)) >= 0);
}

/// `get_borrowed_amount` is non-negative under valid state.
#[rule]
fn borrowed_amount_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    borrowed: i128,
    borrow_index: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&borrowed));
    cvlr_assume!((RAY..=10 * RAY).contains(&borrow_index));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            borrowed,
            borrowed,
            0,
            RAY,
            borrow_index,
            borrowed,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::get_borrowed_amount(e, hub(asset)) >= 0);
}

/// `get_revenue` is non-negative under valid state.
#[rule]
fn protocol_revenue_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    revenue: i128,
    supply_index: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((0..=supplied).contains(&revenue));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=10 * RAY).contains(&supply_index));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            revenue,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::get_revenue(e, hub(asset)) >= 0);
}

/// `get_revenue <= get_supplied_amount` when `revenue <= supplied`.
#[rule]
fn protocol_revenue_le_supplied_view(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    revenue: i128,
    supply_index: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((0..=supplied).contains(&revenue));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=10 * RAY).contains(&supply_index));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            0,
            revenue,
            supply_index,
            RAY,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    let revenue_units = crate::LiquidityPool::get_revenue(e.clone(), hub(asset.clone()));
    let supplied_units = crate::LiquidityPool::get_supplied_amount(e, hub(asset));
    cvlr_assert!(revenue_units <= supplied_units);
}

/// `get_utilisation` is non-negative (upper bound not asserted).
#[rule]
fn capital_utilisation_view_nonneg(
    e: Env,
    admin: Address,
    asset: Address,
    supplied: i128,
    borrowed: i128,
    supply_index: i128,
    borrow_index: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied));
    cvlr_assume!((0..=1_000_000 * RAY).contains(&borrowed));
    cvlr_assume!((SUPPLY_INDEX_FLOOR_RAW..=10 * RAY).contains(&supply_index));
    cvlr_assume!((RAY..=10 * RAY).contains(&borrow_index));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(
            supplied,
            borrowed,
            0,
            supply_index,
            borrow_index,
            supplied,
            e.ledger().timestamp(),
        ),
    );
    cvlr_assert!(crate::LiquidityPool::get_utilisation(e, hub(asset)) >= 0);
}

/// A successful borrow never lends beyond reserves: post-borrow cash stays >= 0
/// (`require_reserves` reverts an over-borrow). `cash` is seeded explicitly and
/// small so the reserve gate — not utilization or caps — is the binding one.
#[rule]
fn borrow_within_reserves(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    cash: i128,
) {
    cvlr_assume!((1..=1_000_000_000_000i128).contains(&amount));
    cvlr_assume!((0..=1_000_000_000_000i128).contains(&cash));
    seed(
        &e,
        admin,
        asset.clone(),
        view_state(100 * RAY, 0, 0, RAY, RAY, cash, e.ledger().timestamp()),
    );

    let _ = borrow_first(&e, caller, action(position(0), amount, asset.clone()));

    cvlr_assert!(crate::LiquidityPool::get_reserves(e, hub(asset)) >= 0);
}
