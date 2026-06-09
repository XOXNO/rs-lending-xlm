//! Liquidity pool ABI wrappers.
//!
//! Pool calls exchange `ScaledPositionRaw` only. Collateral risk parameters
//! remain controller-owned and are merged back after the pool returns scaled
//! supply or debt shares. Position verbs bundle their payload in `PoolAction`,
//! which carries the market asset the central pool routes on.

use common::types::{
    AccountPositionType, InterestRateModel, MarketParamsRaw, MarketStateSnapshot, PoolAction,
    PoolAmountMutation, PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
    ScaledPositionRaw,
};
use soroban_sdk::{Address, Bytes, BytesN, Env};

pub(crate) fn pool_create_market_call(env: &Env, pool_addr: &Address, params: &MarketParamsRaw) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_market(params)
}

pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    action: PoolAction,
    supply_cap: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).supply(&action, &supply_cap)
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    action: PoolAction,
    borrow_cap: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).borrow(&action, &borrow_cap)
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    action: PoolAction,
    fee: i128,
    borrow_cap: i128,
) -> PoolStrategyMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_strategy(
        &action,
        &fee,
        &borrow_cap,
    )
}

pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    action: PoolAction,
    is_liquidation: bool,
    protocol_fee: i128,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).withdraw(
        &action,
        &is_liquidation,
        &protocol_fee,
    )
}

pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    action: PoolAction,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).repay(&action)
}

pub(crate) fn pool_seize_position_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).seize_position(
        asset,
        &side,
        &position,
    )
}

pub(crate) fn pool_flash_loan_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(asset, initiator, receiver, &amount, &fee, data)
}

pub(crate) fn pool_update_indexes_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_indexes(asset)
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
) -> PoolAmountMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).claim_revenue(asset)
}

pub(crate) fn pool_add_rewards_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    amount: i128,
) -> MarketStateSnapshot {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).add_rewards(asset, &amount)
}

pub(crate) fn fetch_pool_sync_data(env: &Env, pool_addr: &Address, asset: &Address) -> PoolSyncData {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).get_sync_data(asset)
}

pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    params: &InterestRateModel,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_params(asset, params)
}

pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}
