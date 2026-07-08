//! LiquidityPool summaries: nondet postconditions within production bounds.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Bytes, Env, Vec};

use common::constants::{RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::types::{
    MarketIndex, MarketParamsRaw, PoolAmountMutation, PoolNetSettleResult, PoolPositionMutation,
    PoolSeizeEntry, PoolStateRaw, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};

fn nondet_market_index() -> MarketIndex {
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index >= RAY);
    MarketIndex {
        supply_index: common::math::fp::Ray::from(supply_index),
        borrow_index: common::math::fp::Ray::from(borrow_index),
    }
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
    }
}

/// Net-settle: nets a supply leg against a debt leg on the same hub-asset,
/// zero token transfer. `0 <= settled_amount <= amount`, both scaled amounts
/// non-increasing, and both legs tied to the single shared settled amount:
/// production `net_settle_one` burns `scaled_withdrawal` from supply and
/// `scaled_repay` from debt, both derived from ONE `gross_amount` (the returned
/// `settled_amount`), so the two legs always move by the identical real amount.
pub fn net_settle_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    supply_position: ScaledPositionRaw,
    debt_position: ScaledPositionRaw,
) -> PoolNetSettleResult {
    // One shared gross real amount drives both legs; draw it once.
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

    // Couple both scaled reductions to the shared settled amount: a zero
    // settlement burns no shares on either leg, so any scaled reduction implies
    // `settled_amount > 0`. This rules out the contradictory states the prior
    // three independent draws permitted (a supply or debt burn while
    // `settled_amount` is zero). It stays a sound over-approximation: a positive
    // settlement may still leave a leg unchanged when its per-index share
    // conversion rounds to zero, so movement is never forced and no exact index
    // math is assumed.
    let supply_unchanged = new_supply_scaled == supply_position.scaled_amount;
    let debt_unchanged = new_debt_scaled == debt_position.scaled_amount;
    cvlr_assume!(settled_amount != 0 || (supply_unchanged && debt_unchanged));

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

/// Flash loan: `amount > 0`, `fee >= 0`, `amount + fee` in range. Returns
/// nothing (production emits an event).
pub fn flash_loan_summary(
    _env: &Env,
    _asset: &Address,
    _initiator: &Address,
    _receiver: &Address,
    amount: i128,
    fee: i128,
    _data: &Bytes,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(fee >= 0);
    cvlr_assume!(fee <= i128::MAX - amount);
}

/// Create strategy: `actual_amount == amount`, `amount_received == amount - fee`, debt non-decreasing.
pub fn create_strategy_summary(
    _env: &Env,
    _asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
    fee: i128,
) -> PoolStrategyMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= position.scaled_amount);
    new_position.scaled_amount = new_scaled;

    cvlr_assume!(fee >= 0);
    cvlr_assume!(amount >= 0);
    cvlr_assume!(fee <= amount);

    let market_index = nondet_market_index();
    PoolStrategyMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        amount_received: amount - fee,
    }
}

/// Seize: no return value; per-entry scaled amounts leave the market totals,
/// the supply index may drop (floored) and the borrow index stays >= RAY, the
/// nondet index semantics of `nondet_market_index`.
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
    cvlr_assume!(borrow_index >= RAY);

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
            is_flashloanable: false,
            flashloan_fee: 0,
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

/// Capital utilisation in RAY, `0 <= util <= RAY`.
pub fn capital_utilisation_summary(_env: &Env) -> i128 {
    // Only `>= 0` is sound: production `capital_utilisation` returns
    // `borrowed/supplied` in RAY, which a bad-debt write-down (borrowed >
    // supplied) pushes above RAY. An upper bound here would exclude that real
    // state and be unsound; consumers must not assume `util <= RAY`.
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
