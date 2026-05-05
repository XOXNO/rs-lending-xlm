use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{AccountPosition, AccountPositionType, MarketParams, PoolKey, PoolState};

fn params(asset: Address) -> MarketParams {
    MarketParams {
        base_borrow_rate_ray: RAY / 100,
        slope1_ray: RAY / 10,
        slope2_ray: RAY / 5,
        slope3_ray: RAY / 2,
        mid_utilization_ray: RAY / 2,
        optimal_utilization_ray: RAY * 8 / 10,
        max_borrow_rate_ray: 2 * RAY,
        reserve_factor_bps: 1_000,
        asset_id: asset,
        asset_decimals: 7,
    }
}

fn state(supplied: i128, borrowed: i128, revenue: i128, timestamp: u64) -> PoolState {
    PoolState {
        supplied_ray: supplied,
        borrowed_ray: borrowed,
        revenue_ray: revenue,
        borrow_index_ray: RAY,
        supply_index_ray: RAY,
        last_timestamp: timestamp * 1000,
    }
}

fn seed(env: &Env, admin: Address, asset: Address, state: PoolState) {
    crate::LiquidityPool::__constructor(env.clone(), admin, params(asset));
    env.storage().instance().set(&PoolKey::State, &state);
}

fn position(scaled: i128) -> AccountPosition {
    AccountPosition {
        scaled_amount_ray: scaled,
        liquidation_threshold_bps: 8_000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 1_000,
        loan_to_value_bps: 7_500,
    }
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
        admin,
        asset,
        state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(RAY);
    let result = crate::LiquidityPool::supply(e, before.clone(), RAY, amount);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
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
        admin,
        asset,
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = crate::LiquidityPool::borrow(e, caller, amount, before.clone(), RAY);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
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
        admin,
        asset,
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = crate::LiquidityPool::withdraw(e, caller, amount, before.clone(), false, 0, RAY);

    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount || result.position.scaled_amount_ray == 0);
    cvlr_assert!(result.position.scaled_amount_ray <= before.scaled_amount_ray);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
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
        asset,
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    let before = position(scaled);
    let result = crate::LiquidityPool::repay(e, caller, amount, before.clone(), RAY);

    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount_ray <= before.scaled_amount_ray);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
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
        asset,
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = crate::LiquidityPool::create_strategy(e, caller, before.clone(), amount, fee, RAY);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.amount_received == amount - fee);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
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
        asset,
        state(100 * RAY, scaled, 0, e.ledger().timestamp()),
    );

    let result =
        crate::LiquidityPool::seize_position(e, AccountPositionType::Borrow, position(scaled), RAY);
    cvlr_assert!(result.scaled_amount_ray == 0);
}

#[rule]
fn claim_revenue_satisfies_controller_summary_contract(e: Env, admin: Address, asset: Address) {
    seed(
        &e,
        admin,
        asset,
        state(100 * RAY, 0, RAY, e.ledger().timestamp()),
    );

    let amount = crate::LiquidityPool::claim_revenue(e, RAY);
    cvlr_assert!(amount >= 0);
}

#[rule]
fn flash_loan_end_satisfies_fee_domain(
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
        admin,
        asset,
        state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    crate::LiquidityPool::flash_loan_begin(e.clone(), amount, receiver.clone());
    crate::LiquidityPool::flash_loan_end(e, amount, fee, receiver);
    cvlr_satisfy!(true);
}
