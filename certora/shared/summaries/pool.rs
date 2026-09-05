use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Bytes, Env, Vec};

use common::constants::{
    BPS, MAX_ASSET_DECIMALS, MAX_BORROW_INDEX_RAY, MAX_BORROW_RATE_RAY, MAX_FLASHLOAN_FEE_BPS,
    MAX_SUPPLY_INDEX_RAY, MIN_ASSET_DECIMALS, RAY, SUPPLY_INDEX_FLOOR_RAW,
};
use common::types::{
    MarketIndex, MarketIndexRaw, MarketParamsRaw, PoolAmountMutation, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStateRaw, PoolStrategyMutation, PoolSyncData,
    ScaledPositionRaw,
};

/// The one generator for a market's pair of indexes.
///
/// Every summary that returns indexes goes through this: the per-verb
/// mutations below, `get_sync_data_summary` (`Context::cached_pool_sync_data`)
/// and `super::bulk_index_summary` (`Context::cached_market_index`). Sharing the
/// generator is what keeps the two `Context` doors from carrying different
/// domains for the same market. Both ends are the pool's own clamps:
/// `update_supply_index` and `apply_bad_debt_to_supply_index` hold the supply
/// index inside `[SUPPLY_INDEX_FLOOR_RAW, MAX_SUPPLY_INDEX_RAY]`, and
/// `update_borrow_index` holds the borrow index inside
/// `[RAY, MAX_BORROW_INDEX_RAY]`.
pub fn nondet_market_index_raw() -> MarketIndexRaw {
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();

    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY);
    cvlr_assume!(borrow_index <= MAX_BORROW_INDEX_RAY);
    MarketIndexRaw {
        supply_index,
        borrow_index,
    }
}

fn nondet_market_index() -> MarketIndex {
    let raw = nondet_market_index_raw();
    MarketIndex {
        supply_index: common::math::fp::Ray::from(raw.supply_index),
        borrow_index: common::math::fp::Ray::from(raw.borrow_index),
    }
}

/// `MarketParamsRaw::verify` rejects anything above `WAD_DECIMALS`, and market
/// creation rejects anything below `MIN_ASSET_DECIMALS`.
fn nondet_asset_decimals() -> u32 {
    let asset_decimals: u32 = nondet();
    cvlr_assume!((MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&asset_decimals));
    asset_decimals
}

pub fn supply_summary(
    _env: &Env,
    _asset: &Address,
    position: ScaledPositionRaw,
    amount: i128,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    // The pool mints shares only when the movement changes the scaled record;
    // positive supply strictly grows the position (dust reverts inside the pool).
    if amount > 0 {
        cvlr_assume!(new_scaled > position.scaled_amount);
    } else {
        cvlr_assume!(new_scaled == position.scaled_amount);
    }
    new_position.scaled_amount = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

pub fn borrow_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    // Borrow mints debt shares only when the movement changes the scaled
    // record; positive borrow strictly grows the position.
    if amount > 0 {
        cvlr_assume!(new_scaled > position.scaled_amount);
    } else {
        cvlr_assume!(new_scaled == position.scaled_amount);
    }
    new_position.scaled_amount = new_scaled;

    let market_index = nondet_market_index();
    PoolPositionMutation {
        position: new_position,
        market_index: (&market_index).into(),
        actual_amount: amount,
        asset_decimals: nondet_asset_decimals(),
    }
}

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
    // A successful withdraw burns shares: strictly when the position existed,
    // otherwise it must be a zero-amount no-op (full closes land on zero).
    if amount > 0 && position.scaled_amount > 0 {
        cvlr_assume!(new_scaled >= 0);
        cvlr_assume!(new_scaled < position.scaled_amount);
    } else {
        cvlr_assume!(new_scaled == position.scaled_amount);
    }
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

pub fn repay_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    let mut new_position = position.clone();
    let new_scaled: i128 = nondet();
    // A successful repayment burns debt shares: strictly when the position
    // existed, otherwise it must be a zero-amount no-op (full closes land on
    // zero, partial repays burn at least one share or revert).
    if amount > 0 && position.scaled_amount > 0 {
        cvlr_assume!(new_scaled >= 0);
        cvlr_assume!(new_scaled < position.scaled_amount);
    } else {
        cvlr_assume!(new_scaled == position.scaled_amount);
    }
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

pub fn net_settle_summary(
    _env: &Env,
    _asset: &Address,
    amount: i128,
    supply_position: ScaledPositionRaw,
    debt_position: ScaledPositionRaw,
) -> PoolNetSettleResult {
    let settled_amount: i128 = nondet();
    cvlr_assume!(settled_amount >= 0);
    cvlr_assume!(settled_amount <= amount);

    let mut new_supply = supply_position.clone();
    let mut new_debt = debt_position.clone();
    let new_supply_scaled: i128 = nondet();
    let new_debt_scaled: i128 = nondet();
    cvlr_assume!(new_supply_scaled >= 0);
    cvlr_assume!(new_supply_scaled <= supply_position.scaled_amount);
    cvlr_assume!(new_debt_scaled >= 0);
    cvlr_assume!(new_debt_scaled <= debt_position.scaled_amount);
    // Production resolve_net_settle: a positive settlement burns strictly on
    // both sides (pool enforces burn>0 whenever gross_amount>0); a zero
    // settlement leaves both positions untouched.
    if settled_amount > 0 {
        cvlr_assume!(new_supply_scaled < supply_position.scaled_amount);
        cvlr_assume!(new_debt_scaled < debt_position.scaled_amount);
    } else {
        cvlr_assume!(new_supply_scaled == supply_position.scaled_amount);
        cvlr_assume!(new_debt_scaled == debt_position.scaled_amount);
    }
    new_supply.scaled_amount = new_supply_scaled;

    new_debt.scaled_amount = new_debt_scaled;

    let market_index = nondet_market_index();
    PoolNetSettleResult {
        supply_position: new_supply,
        debt_position: new_debt,
        market_index: (&market_index).into(),
        settled_amount,
    }
}

pub fn update_indexes_summary(_env: &Env, _asset: &Address) {}

pub fn recapitalize_summary(_env: &Env, _asset: &Address, amount: i128) -> PoolAmountMutation {
    let actual_amount: i128 = nondet();
    cvlr_assume!(actual_amount >= 0);
    cvlr_assume!(actual_amount <= amount);
    PoolAmountMutation { actual_amount }
}

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
    // Production flash fee is amount * fee_bps / BPS (half-up) with
    // fee_bps <= MAX_FLASHLOAN_FEE_BPS (500), so fee <= amount always.
    // Bound the summary to the production-faithful range: strictly wider
    // ranges only inflate the SMT search for the flash-loan rules.
    cvlr_assume!(fee <= amount);
    fee
}

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

pub fn seize_positions_summary(_env: &Env, _entries: &Vec<PoolSeizeEntry>) {}

pub fn claim_revenue_summary(_env: &Env, _asset: &Address) -> PoolAmountMutation {
    let amount: i128 = nondet();
    cvlr_assume!(amount >= 0);
    PoolAmountMutation {
        actual_amount: amount,
    }
}

/// Market snapshot returned by `LiquidityPool::get_sync_data`, feeding
/// `Context::cached_pool_sync_data`.
///
/// The index fields come from [`nondet_market_index_raw`], the same generator
/// [`super::bulk_index_summary`] uses, so a rule that reads both doors for one
/// market cannot be handed two different index domains. The controller harness
/// memoises both per rule (`certora/controller/harness/ghost_prices.rs`), which
/// is what makes the two reads agree on a *value* and not merely on a domain.
///
/// The rate-model fields mirror `InterestRateModel::verify`
/// (`common/src/types/pool.rs`) exactly, because a stored market cannot hold a
/// curve that failed it: non-negative base, monotone slopes below
/// `max_borrow_rate`, `max_borrow_rate` in `(base, MAX_BORROW_RATE_RAY]`,
/// `0 < mid_utilization < optimal_utilization < RAY`, `optimal_utilization <=
/// max_utilization <= RAY`, `reserve_factor < BPS`, and `flashloan_fee <=
/// MAX_FLASHLOAN_FEE_BPS`. Drawing them unconstrained (the previous form) let
/// every controller rule that reads a market see a curve the pool would have
/// rejected — negative slopes, an inverted kink, or a rate above the ceiling —
/// and made `calculate_borrow_rate` a nonlinear query over the full `i128` box.
///
/// The state fields stay as they are: `supplied`, `borrowed`, `revenue` and
/// `cash` are non-negative and otherwise unconstrained, which is the strong
/// form for the frame rules that read them.
pub fn get_sync_data_summary(_env: &Env, asset: &Address) -> PoolSyncData {
    let supplied: i128 = nondet();
    let borrowed: i128 = nondet();
    let revenue: i128 = nondet();
    let cash: i128 = nondet();
    let last_timestamp: u64 = nondet();

    cvlr_assume!(supplied >= 0);
    cvlr_assume!(borrowed >= 0);
    cvlr_assume!(revenue >= 0);
    cvlr_assume!(cash >= 0);

    let MarketIndexRaw {
        supply_index,
        borrow_index,
    } = nondet_market_index_raw();

    let max_borrow_rate: i128 = nondet();
    let base_borrow_rate: i128 = nondet();
    let slope1: i128 = nondet();
    let slope2: i128 = nondet();
    let slope3: i128 = nondet();
    let mid_utilization: i128 = nondet();
    let optimal_utilization: i128 = nondet();
    let max_utilization: i128 = nondet();
    cvlr_assume!(base_borrow_rate >= 0);
    cvlr_assume!(base_borrow_rate <= slope1);
    cvlr_assume!(slope1 <= slope2);
    cvlr_assume!(slope2 <= slope3);
    cvlr_assume!(slope3 <= max_borrow_rate);
    cvlr_assume!(max_borrow_rate > base_borrow_rate);
    cvlr_assume!(max_borrow_rate <= MAX_BORROW_RATE_RAY);
    cvlr_assume!(mid_utilization > 0);
    cvlr_assume!(mid_utilization < optimal_utilization);
    cvlr_assume!(optimal_utilization < RAY);
    cvlr_assume!(optimal_utilization <= max_utilization);
    cvlr_assume!(max_utilization <= RAY);
    let reserve_factor: u32 = nondet();
    cvlr_assume!(i128::from(reserve_factor) < BPS);
    let asset_decimals: u32 = nondet_asset_decimals();
    let is_flashloanable: bool = nondet();
    let flashloan_fee: u32 = nondet();
    cvlr_assume!(i128::from(flashloan_fee) <= MAX_FLASHLOAN_FEE_BPS);
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
