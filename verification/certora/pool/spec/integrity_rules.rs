use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::{BPS, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::fp::Ray;
use common::types::{AccountPosition, AccountPositionType, MarketParams, PoolKey, PoolState};

fn valid_params(asset: Address) -> MarketParams {
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

fn valid_state(supplied: i128, borrowed: i128, revenue: i128, timestamp: u64) -> PoolState {
    PoolState {
        supplied_ray: supplied,
        borrowed_ray: borrowed,
        revenue_ray: revenue,
        borrow_index_ray: RAY,
        supply_index_ray: RAY,
        last_timestamp: timestamp * 1000,
    }
}

fn seed_pool(env: &Env, admin: Address, asset: Address, state: PoolState) {
    crate::LiquidityPool::__constructor(env.clone(), admin, valid_params(asset));
    env.storage().instance().set(&PoolKey::State, &state);
}

fn read_state(env: &Env) -> PoolState {
    env.storage().instance().get(&PoolKey::State).unwrap()
}

fn position(scaled_amount_ray: i128) -> AccountPosition {
    AccountPosition {
        scaled_amount_ray,
        liquidation_threshold_bps: 8_000,
        liquidation_bonus_bps: 500,
        liquidation_fees_bps: 1_000,
        loan_to_value_bps: 7_500,
    }
}

#[rule]
fn constructor_initializes_valid_state(e: Env, admin: Address, asset: Address) {
    crate::LiquidityPool::__constructor(e.clone(), admin, valid_params(asset));

    let state = read_state(&e);
    cvlr_assert!(state.supplied_ray == 0);
    cvlr_assert!(state.borrowed_ray == 0);
    cvlr_assert!(state.revenue_ray == 0);
    cvlr_assert!(state.borrow_index_ray == RAY);
    cvlr_assert!(state.supply_index_ray == RAY);
}

#[rule]
fn pool_state_domain_invariant(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(100 * RAY, 25 * RAY, RAY, e.ledger().timestamp()),
    );

    let state = read_state(&e);
    cvlr_assert!(state.supplied_ray >= 0);
    cvlr_assert!(state.borrowed_ray >= 0);
    cvlr_assert!(state.revenue_ray >= 0);
    cvlr_assert!(state.borrow_index_ray >= RAY);
    cvlr_assert!(state.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn supply_preserves_nonnegative_state(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(RAY);
    let result = crate::LiquidityPool::supply(e.clone(), before.clone(), RAY, amount);
    let state = read_state(&e);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(state.supplied_ray >= 0);
    cvlr_assert!(state.borrowed_ray >= 0);
    cvlr_assert!(state.revenue_ray >= 0);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn borrow_preserves_nonnegative_state(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(0);
    let result = crate::LiquidityPool::borrow(e.clone(), caller, amount, before.clone(), RAY);
    let state = read_state(&e);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount_ray >= before.scaled_amount_ray);
    cvlr_assert!(state.supplied_ray >= 0);
    cvlr_assert!(state.borrowed_ray >= 0);
    cvlr_assert!(result.market_index.borrow_index_ray >= RAY);
    cvlr_assert!(result.market_index.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn withdraw_never_creates_negative_position(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(scaled_before);
    let result = crate::LiquidityPool::withdraw(e.clone(), caller, amount, before, false, 0, RAY);
    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
}

#[rule]
fn repay_never_creates_negative_debt(
    e: Env,
    admin: Address,
    asset: Address,
    caller: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let before = position(scaled_before);
    let result = crate::LiquidityPool::repay(e.clone(), caller, amount, before, RAY);
    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount_ray >= 0);
}

#[rule]
fn bad_debt_socialization_keeps_supply_index_above_floor(
    e: Env,
    admin: Address,
    asset: Address,
    bad_debt: i128,
) {
    cvlr_assume!((0..=200 * RAY).contains(&bad_debt));
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(100 * RAY, 10 * RAY, 0, e.ledger().timestamp()),
    );

    let mut cache = crate::cache::Cache::load(&e);
    crate::interest::apply_bad_debt_to_supply_index(&mut cache, Ray::from_raw(bad_debt));
    cvlr_assert!(cache.supply_index.raw() >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn seize_position_zeroes_scaled_amount(
    e: Env,
    admin: Address,
    asset: Address,
    scaled_before: i128,
) {
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let after = crate::LiquidityPool::seize_position(
        e,
        AccountPositionType::Borrow,
        position(scaled_before),
        RAY,
    );
    cvlr_assert!(after.scaled_amount_ray == 0);
}

#[rule]
fn update_params_keeps_rate_domain(
    e: Env,
    admin: Address,
    asset: Address,
    base: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    max_rate: i128,
) {
    cvlr_assume!((0..=RAY / 10).contains(&base));
    cvlr_assume!((base..=RAY / 2).contains(&slope1));
    cvlr_assume!((slope1..=RAY).contains(&slope2));
    cvlr_assume!((slope2..=2 * RAY).contains(&slope3));
    cvlr_assume!(max_rate > base && max_rate >= slope3 && max_rate <= 2 * RAY);
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(0, 0, 0, e.ledger().timestamp()),
    );

    crate::LiquidityPool::update_params(
        e.clone(),
        max_rate,
        base,
        slope1,
        slope2,
        slope3,
        RAY / 2,
        RAY * 8 / 10,
        (BPS / 10) as u32,
    );

    let params: MarketParams = e.storage().instance().get(&PoolKey::Params).unwrap();
    cvlr_assert!(params.max_borrow_rate_ray == max_rate);
    cvlr_assert!(params.base_borrow_rate_ray == base);
    cvlr_assert!(params.slope1_ray == slope1);
    cvlr_assert!(params.slope2_ray == slope2);
    cvlr_assert!(params.slope3_ray == slope3);
}

#[rule]
fn pool_integrity_reachability(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset,
        valid_state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );
    let state = read_state(&e);
    cvlr_satisfy!(state.supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
}
