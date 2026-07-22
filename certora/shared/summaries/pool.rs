//! Trusted LiquidityPool abstractions for controller-only jobs.
//!
//! These summaries do not constitute a composed pool proof. Several erase
//! pool storage, token-transfer, batch-order, or callback effects, so rules
//! using them must stay scoped to controller-local output/control-flow claims.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Bytes, Env, Vec};

use common::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    MarketIndex, MarketParamsRaw, PoolAmountMutation, PoolNetSettleResult, PoolPositionMutation,
    PoolSeizeEntry, PoolStateRaw, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};

fn nondet_market_index() -> MarketIndex {
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();
    // Production band: floor from bad-debt write-down, caps from update_*_index
    // clamps (proved by update_supply_index_capped / update_borrow_index_capped).
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY);
    cvlr_assume!(borrow_index <= MAX_BORROW_INDEX_RAY);
    MarketIndex {
        supply_index: common::math::fp::Ray::from(supply_index),
        borrow_index: common::math::fp::Ray::from(borrow_index),
    }
}

/// Immutable market decimals carried by every pool mutation.
/// Production validates this field against the RAY decimal domain.
fn nondet_asset_decimals() -> u32 {
    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);
    asset_decimals
}

/// Nondet index with supply and borrow indexes >= prior (except seize_positions supply drop).
fn nondet_market_index_monotone(prior: &MarketIndex) -> MarketIndex {
    let idx = nondet_market_index();
    cvlr_assume!(idx.supply_index >= prior.supply_index);
    cvlr_assume!(idx.borrow_index >= prior.borrow_index);
    idx
}

/// Per-entry supply: `actual_amount == amount`, scaled amount non-decreasing, valid indexes.
pub fn supply_summary(
    _env: &Env,
    _asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

/// Per-entry borrow: `actual_amount == amount`, scaled amount non-decreasing, valid indexes.
pub fn borrow_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

/// Withdraw: `0 <= actual_amount <= amount`, scaled amount non-increasing, valid indexes.
pub fn withdraw_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
    _is_liquidation: bool,
    _protocol_fee: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

/// Repay: `0 <= actual_amount <= amount`, scaled amount non-increasing, valid indexes.
pub fn repay_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0);
    cvlr_assume!(new_scaled <= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

/// Net-settle: `0 <= settled_amount <= amount`, both scaled non-increasing.
/// Production burns both legs from one shared `gross_amount` (= `settled_amount`).
pub fn net_settle_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    supply_position: ScaledPositionRaw,
    debt_position: ScaledPositionRaw,
) -> PoolNetSettleResult {
    // One shared gross amount drives both legs.
    let settled_amount: i128 = nondet();
    cvlr_assume!(settled_amount >= 0);
    cvlr_assume!(settled_amount <= amount);

    let mut new_supply = supply_position.clone();
    let new_supply_scaled: i128 = nondet();
    cvlr_assume!(new_supply_scaled >= 0);
    cvlr_assume!(new_supply_scaled <= supply_position.scaled_amount);
    new_supply.scaled_amount = new_supply_scaled;

    let mut new_debt = debt_position.clone();
    let new_debt_scaled: i128 = nondet();
    cvlr_assume!(new_debt_scaled >= 0);
    cvlr_assume!(new_debt_scaled <= debt_position.scaled_amount);
    new_debt.scaled_amount = new_debt_scaled;

    // Do not relate a zero token settlement to unchanged shares. Production
    // may burn a dust position whose floored token amount is zero.

    let market_index = nondet_market_index();
    PoolNetSettleResult {
        supply_position: new_supply,
        debt_position: new_debt,
        market_index: (&market_index).into(),
        settled_amount,
    }
}

/// Index sync: production accrues + emits an event and returns nothing.
pub fn update_indexes_summary(_env: &Env, _asset: &Address) {}

/// Add rewards: non-negative amount; empty pool panics in production. Returns
/// nothing (production emits an event).
pub fn add_rewards_summary(_env: &Env, _asset: &Address, _amount: i128) {}

/// Flash-loan return abstraction for controller lock cleanup only. It omits
/// liquidity checks, transfers, receiver callback, repayment, and pool state.
pub fn flash_loan_summary(
    _env: &Env,
    _asset: &Address,
    _initiator: &Address,
    _receiver: &Address,
    amount: i128,
    _data: &Bytes,
) -> i128 {
    let fee: i128 = nondet();
    cvlr_assume!(amount > 0);
    cvlr_assume!(fee >= 0);
    cvlr_assume!(fee <= i128::MAX - amount);
    fee
}

/// Create strategy: debt non-decreasing; fee-free migrations receive the full
/// amount, while fee-charging calls admit every valid configured fee.
pub fn create_strategy_summary(
    _env: &Env,
    _asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    charge_fee: bool,
) -> PoolStrategyMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    cvlr_assume!(amount >= 0);
    let fee: i128 = if charge_fee { nondet() } else { 0 };
    cvlr_assume!(fee >= 0);
    cvlr_assume!(fee <= amount);

    let market_index = nondet_market_index();
    PoolStrategyMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        amount_received: amount - fee,
        asset_decimals: nondet_asset_decimals(),
    }
}

/// Seize: scaled amounts leave market totals; supply index may drop (floored);
/// borrow index >= RAY (`nondet_market_index` bounds).
pub fn seize_positions_summary(_env: &Env, _entries: &Vec<PoolSeizeEntry>) {}

/// Claim revenue: non-negative transfer amount.
pub fn claim_revenue_summary(_env: &Env, _asset: &Address) -> PoolAmountMutation {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    PoolAmountMutation {
        actual_amount: amount,
    }
}

/// Sync data: state fields non-negative with valid indexes; params fully nondet.
pub fn get_sync_data_summary(_env: &Env, asset: &Address) -> PoolSyncData {
    let supplied: i128 = nondet();
    let borrowed: i128 = nondet();
    let revenue: i128 = nondet();
    let cash: i128 = nondet();
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();
    let last_timestamp: u64 = nondet();

    cvlr_assume!(supplied >= 0);
    cvlr_assume!(borrowed >= 0);
    cvlr_assume!(revenue >= 0);
    cvlr_assume!(cash >= 0);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY);
    cvlr_assume!(borrow_index <= MAX_BORROW_INDEX_RAY);

    let max_borrow_rate: i128 = nondet();
    let base_borrow_rate: i128 = nondet();
    let slope1: i128 = nondet();
    let slope2: i128 = nondet();
    let slope3: i128 = nondet();
    let mid_utilization: i128 = nondet();
    let optimal_utilization: i128 = nondet();
    let max_utilization: i128 = nondet();
    let reserve_factor: u32 = nondet();
    cvlr_assume!(i128::from(reserve_factor) < common::constants::BPS);
    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);
    let is_flashloanable: bool = nondet();
    let flashloan_fee: u32 = nondet();
    cvlr_assume!(i128::from(flashloan_fee) <= common::constants::BPS);
    let asset_id: Address = asset.clone();

    PoolSyncData {
        params: MarketParamsRaw {
            max_borrow_rate,
            base_borrow_rate,
            slope1,
            slope2,
            slope3,
            mid_utilization,
            optimal_utilization,
            max_utilization,
            reserve_factor,
            is_flashloanable,
            flashloan_fee,
            asset_id,
            asset_decimals,
        },
        state: PoolStateRaw {
            supplied,
            borrowed,
            revenue,
            borrow_index,
            supply_index,
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

/// Capital utilisation in RAY, `util >= 0` only (may exceed RAY after bad-debt write-down).
pub fn capital_utilisation_summary(_env: &Env) -> i128 {
    // No upper bound: production util can exceed RAY when borrowed > supplied.
    let util_ray: i128 = nondet();
    cvlr_assume!(util_ray >= 0);
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
