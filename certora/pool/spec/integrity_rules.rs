use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::{BPS, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::types::{
    AccountPositionType, HubAssetKey, InterestRateModel, MarketParamsRaw, PoolAction, PoolKey,
    PoolStateRaw, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;

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
        supply_cap: 0,
        borrow_cap: 0,
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

// Bulk-of-one wrappers: one entry through the bulk endpoint.
fn supply_first(e: &Env, act: PoolAction, _cap: i128) -> common::types::PoolPositionMutation {
    let mut entries: soroban_sdk::Vec<common::types::PoolSupplyEntry> = soroban_sdk::Vec::new(e);
    entries.push_back(common::types::PoolSupplyEntry { action: act });
    crate::LiquidityPool::supply(e.clone(), entries).get_unchecked(0)
}

fn borrow_first(
    e: &Env,
    receiver: Address,
    act: PoolAction,
    _cap: i128,
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

#[rule]
fn create_market_initializes_valid_state(e: Env, admin: Address, asset: Address) {
    crate::LiquidityPool::__constructor(e.clone(), admin);
    crate::LiquidityPool::create_market(e.clone(), 0, valid_params(asset.clone()));

    let state = read_state(&e, &asset);
    cvlr_assert!(state.supplied == 0);
    cvlr_assert!(state.borrowed == 0);
    cvlr_assert!(state.revenue == 0);
    cvlr_assert!(state.borrow_index == RAY);
    cvlr_assert!(state.supply_index == RAY);
}

#[rule]
fn pool_state_domain_invariant(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 25 * RAY, RAY, e.ledger().timestamp()),
    );

    let state = read_state(&e, &asset);
    cvlr_assert!(state.supplied >= 0);
    cvlr_assert!(state.borrowed >= 0);
    cvlr_assert!(state.revenue >= 0);
    cvlr_assert!(state.borrow_index >= RAY);
    cvlr_assert!(state.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
}

#[rule]
fn supply_preserves_nonnegative_state(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000i128);
    seed_pool(
        &e,
        admin.clone(),
        asset.clone(),
        valid_state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(RAY);
    let result = supply_first(&e, action(before.clone(), amount, asset.clone()), i128::MAX);
    let state = read_state(&e, &asset);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount >= before.scaled_amount);
    cvlr_assert!(state.supplied >= 0);
    cvlr_assert!(state.borrowed >= 0);
    cvlr_assert!(state.revenue >= 0);
    cvlr_assert!(result.market_index.borrow_index >= RAY);
    cvlr_assert!(result.market_index.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
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
    let result = borrow_first(
        &e,
        caller,
        action(before.clone(), amount, asset.clone()),
        i128::MAX,
    );
    let state = read_state(&e, &asset);

    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount >= before.scaled_amount);
    cvlr_assert!(state.supplied >= 0);
    cvlr_assert!(state.borrowed >= 0);
    cvlr_assert!(result.market_index.borrow_index >= RAY);
    cvlr_assert!(result.market_index.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
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
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let before = position(scaled_before);
    let result = withdraw_first(&e, caller, action(before, amount, asset), false, 0);
    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.position.scaled_amount >= 0);
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
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let before = position(scaled_before);
    let result = repay_first(&e, caller, action(before, amount, asset));
    cvlr_assert!(result.actual_amount >= 0);
    cvlr_assert!(result.actual_amount <= amount);
    cvlr_assert!(result.position.scaled_amount >= 0);
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
        asset.clone(),
        valid_state(100 * RAY, 10 * RAY, 0, e.ledger().timestamp()),
    );

    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::interest::apply_bad_debt_to_supply_index(&mut cache, Ray::from(bad_debt));
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
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let after = crate::LiquidityPool::seize_position(
        e,
        hub(asset),
        AccountPositionType::Borrow,
        position(scaled_before),
    );
    cvlr_assert!(after.position.scaled_amount == 0);
}

#[rule]
#[allow(clippy::too_many_arguments)]
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
        asset.clone(),
        valid_state(0, 0, 0, e.ledger().timestamp()),
    );

    let model = InterestRateModel {
        max_borrow_rate: max_rate,
        base_borrow_rate: base,
        slope1,
        slope2,
        slope3,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY * 95 / 100,
        reserve_factor: (BPS / 10) as u32,
    };

    crate::LiquidityPool::update_params(e.clone(), hub(asset.clone()), model);

    let params: MarketParamsRaw = e
        .storage()
        .persistent()
        .get(&PoolKey::Params(hub(asset)))
        .unwrap();
    cvlr_assert!(params.max_borrow_rate == max_rate);
    cvlr_assert!(params.base_borrow_rate == base);
    cvlr_assert!(params.slope1 == slope1);
    cvlr_assert!(params.slope2 == slope2);
    cvlr_assert!(params.slope3 == slope3);
}

#[rule]
fn pool_integrity_reachability(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(10 * RAY, 0, 0, e.ledger().timestamp()),
    );
    let state = read_state(&e, &asset);
    cvlr_satisfy!(state.supply_index >= SUPPLY_INDEX_FLOOR_RAW);
}

/// `add_rewards` preserves `revenue <= supplied`.
#[rule]
fn revenue_le_supplied_after_add_rewards(
    e: Env,
    admin: Address,
    asset: Address,
    supplied_init: i128,
    revenue_init: i128,
    rewards: i128,
) {
    cvlr_assume!((0..=1_000_000 * RAY).contains(&supplied_init));
    cvlr_assume!((0..=supplied_init).contains(&revenue_init));
    cvlr_assume!((0..=1_000_000).contains(&rewards));

    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(supplied_init, 0, revenue_init, e.ledger().timestamp()),
    );

    crate::LiquidityPool::add_rewards(e.clone(), hub(asset.clone()), rewards);

    let state = read_state(&e, &asset);
    cvlr_assert!(state.revenue <= state.supplied);
    cvlr_assert!(state.revenue >= 0);
}

/// Flash-loan fees preserve `revenue <= supplied`.
#[rule]
fn flash_loan_revenue_supplied_lockstep(
    e: Env,
    admin: Address,
    asset: Address,
    supplied_init: i128,
    revenue_init: i128,
) {
    cvlr_assume!(supplied_init > 0 && supplied_init <= 1_000_000 * RAY);
    cvlr_assume!(revenue_init >= 0 && revenue_init <= supplied_init);

    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(supplied_init, 0, revenue_init, e.ledger().timestamp()),
    );

    let fee = Ray::from(1_000_000);
    let mut cache = crate::cache::Cache::load(&e, &hub(asset.clone()));
    crate::interest::add_protocol_revenue(&mut cache, fee);
    cache.save();

    let state = read_state(&e, &asset);
    cvlr_assert!(state.revenue <= state.supplied);
    cvlr_assert!(state.revenue >= 0);
}
