//! LiquidityPool summaries: nondet postconditions within production bounds.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Bytes, Env};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    AccountPositionType, MarketIndex, MarketParamsRaw, MarketStateSnapshot, PoolAmountMutation,
    PoolPositionMutation, PoolStateRaw, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};

fn nondet_market_index() -> MarketIndex {
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);
    MarketIndex {
        supply_index: common::math::fp::Ray::from(supply_index_ray),
        borrow_index: common::math::fp::Ray::from(borrow_index_ray),
    }
}

/// Nondet index with supply and borrow indexes >= prior (except seize_position supply drop).
fn nondet_market_index_monotone(prior: &MarketIndex) -> MarketIndex {
    let idx = nondet_market_index();
    cvlr_assume!(idx.supply_index >= prior.supply_index);
    cvlr_assume!(idx.borrow_index >= prior.borrow_index);
    idx
}

fn nondet_market_state(asset: &Address, market_index: &MarketIndex) -> MarketStateSnapshot {
    let timestamp: u64 = nondet();
    let reserves_ray: i128 = nondet();
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let revenue_ray: i128 = nondet();
    cvlr_assume!(reserves_ray >= 0);
    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(revenue_ray >= 0);
    MarketStateSnapshot {
        asset: asset.clone(),
        timestamp,
        supply_index_ray: market_index.supply_index.raw(),
        borrow_index_ray: market_index.borrow_index.raw(),
        reserves_ray,
        supplied_ray,
        borrowed_ray,
        revenue_ray,
        asset_price_wad: None,
    }
}

/// Per-entry supply: `actual_amount == amount`, scaled amount non-decreasing, valid indexes.
pub fn supply_summary(
    _env: &Env,
    asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    _supply_cap: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: amount,
    }
}

/// Per-entry borrow: `actual_amount == amount`, scaled amount non-decreasing, valid indexes.
pub fn borrow_summary(
    _env: &Env,
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
    _borrow_cap: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: amount,
    }
}

/// Withdraw: `0 <= actual_amount <= amount`, scaled amount non-increasing, valid indexes.
pub fn withdraw_summary(
    _env: &Env,
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
    _is_liquidation: bool,
    _protocol_fee: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount,
    }
}

/// Repay: `0 <= actual_amount <= amount`, scaled amount non-increasing, valid indexes.
pub fn repay_summary(
    _env: &Env,
    asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount,
    }
}

/// Index sync: fresh market state with valid supply/borrow indexes.
pub fn update_indexes_summary(_env: &Env, asset: &Address) -> MarketStateSnapshot {
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
}

/// Add rewards: non-negative amount; empty pool panics in production.
pub fn add_rewards_summary(_env: &Env, asset: &Address, _amount: i128) -> MarketStateSnapshot {
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
}

/// Flash loan: `amount > 0`, `fee >= 0`, `amount + fee` in range; fee added to revenue.
pub fn flash_loan_summary(
    _env: &Env,
    asset: &Address,
    _initiator: &Address,
    _receiver: &Address,
    amount: i128,
    fee: i128,
    _data: &Bytes,
) -> MarketStateSnapshot {
    cvlr_assume!(amount > 0);
    cvlr_assume!(fee >= 0);
    cvlr_assume!(fee <= i128::MAX - amount);
    let market_index = nondet_market_index();
    nondet_market_state(asset, &market_index)
}

/// Create strategy: `actual_amount == amount`, `amount_received == amount - fee`, debt non-decreasing.
pub fn create_strategy_summary(
    _env: &Env,
    asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    fee: i128,
    _borrow_cap: i128,
) -> PoolStrategyMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount_ray);
    new_position.scaled_amount_ray = new_scaled;

    cvlr_assume!(fee >= 0);
    cvlr_assume!(amount >= 0);
    cvlr_assume!(fee <= amount);

    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolStrategyMutation {
        position: new_position,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: amount,
        amount_received: amount - fee,
    }
}

/// Seize: scaled amount zeroed; supply index may drop (floored), borrow index >= RAY.
pub fn seize_position_summary(
    _env: &Env,
    asset: &Address,
    _side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut zeroed = position.clone();
    zeroed.scaled_amount_ray = 0;
    let market_index = nondet_market_index();
    let market_state = nondet_market_state(asset, &market_index);
    PoolPositionMutation {
        position: zeroed,
        market_index: (&market_index).into(),
        market_state,
        actual_amount: 0,
    }
}

/// Claim revenue: non-negative transfer amount.
pub fn claim_revenue_summary(_env: &Env, asset: &Address) -> PoolAmountMutation {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    let market_index = nondet_market_index();
    PoolAmountMutation {
        market_state: nondet_market_state(asset, &market_index),
        actual_amount: amount,
    }
}

/// Sync data: state fields non-negative with valid indexes; params fully nondet.
pub fn get_sync_data_summary(_env: &Env, asset: &Address) -> PoolSyncData {
    let supplied_ray: i128 = nondet();
    let borrowed_ray: i128 = nondet();
    let revenue_ray: i128 = nondet();
    let cash: i128 = nondet();
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    let last_timestamp: u64 = nondet();

    cvlr_assume!(supplied_ray >= 0);
    cvlr_assume!(borrowed_ray >= 0);
    cvlr_assume!(revenue_ray >= 0);
    cvlr_assume!(cash >= 0);
    cvlr_assume!(supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= RAY);

    let max_borrow_rate_ray: i128 = nondet();
    let base_borrow_rate_ray: i128 = nondet();
    let slope1_ray: i128 = nondet();
    let slope2_ray: i128 = nondet();
    let slope3_ray: i128 = nondet();
    let mid_utilization_ray: i128 = nondet();
    let optimal_utilization_ray: i128 = nondet();
    let max_utilization_ray: i128 = nondet();
    let reserve_factor_bps: u32 = nondet();
    cvlr_assume!(i128::from(reserve_factor_bps) < common::constants::BPS);
    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);
    let asset_id: Address = asset.clone();

    PoolSyncData {
        params: MarketParamsRaw {
            max_borrow_rate_ray,
            base_borrow_rate_ray,
            slope1_ray,
            slope2_ray,
            slope3_ray,
            mid_utilization_ray,
            optimal_utilization_ray,
            max_utilization_ray,
            reserve_factor_bps,
            asset_id,
            asset_decimals,
        },
        state: PoolStateRaw {
            supplied_ray,
            borrowed_ray,
            revenue_ray,
            borrow_index_ray,
            supply_index_ray,
            last_timestamp,
            cash,
        },
    }
}

/// Accounted `cash` (pool state), non-negative.
pub fn reserves_summary(_env: &Env) -> i128 {
    let cash: i128 = nondet();
    cvlr_assume!(cash >= 0);
    cash
}

/// Rescaled supplied amount, non-negative.
pub fn supplied_amount_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Rescaled borrowed amount, non-negative.
pub fn borrowed_amount_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Rescaled protocol revenue, non-negative.
pub fn protocol_revenue_summary(_env: &Env) -> i128 {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    amount
}

/// Capital utilisation in RAY, `0 <= util <= RAY`.
pub fn capital_utilisation_summary(_env: &Env) -> i128 {
    let util_ray: i128 = nondet();
    cvlr_assume!(util_ray >= 0);
    cvlr_assume!(util_ray <= RAY);
    util_ray
}

/// Four pool quantity views for cross-view rules.
pub struct PoolViewsSnapshot {
    pub reserves: i128,
    pub supplied: i128,
    pub borrowed: i128,
    pub revenue: i128,
}

/// Joint snapshot: each value >= 0, `revenue <= supplied`, `borrowed <= supplied + revenue`.
pub fn pool_snapshot_summary(_env: &Env) -> PoolViewsSnapshot {
    let reserves: i128 = nondet();
    let supplied: i128 = nondet();
    let borrowed: i128 = nondet();
    let revenue: i128 = nondet();
    cvlr_assume!(reserves >= 0);
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(borrowed >= 0);
    cvlr_assume!(revenue >= 0);
    cvlr_assume!(revenue <= supplied);
    cvlr_assume!(borrowed <= supplied + revenue);
    PoolViewsSnapshot {
        reserves,
        supplied,
        borrowed,
        revenue,
    }
}

/// Monotone index from prior snapshot for rules outside typed summaries.
pub fn fresh_monotone_index(prior: &MarketIndex) -> MarketIndex {
    nondet_market_index_monotone(prior)
}