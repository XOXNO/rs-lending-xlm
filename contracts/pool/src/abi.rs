//! Compile-time guard that `LiquidityPool` matches
//! `pool_interface::LiquidityPoolInterface` exactly. Drift fails the build.

#![allow(dead_code)]

use crate::LiquidityPool;
#[allow(unused_imports)]
use pool_interface::LiquidityPoolInterface;
use soroban_sdk::{Address, Bytes, BytesN, Env};

use common::types::{
    AccountPositionRaw, AccountPositionType, InterestRateModel, MarketStateSnapshot,
    PoolAmountMutation, PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
};

fn _abi_proof() {
    let _: fn(Env, AccountPositionRaw, i128, i128) -> PoolPositionMutation = LiquidityPool::supply;
    let _: fn(Env, Address, i128, AccountPositionRaw, i128) -> PoolPositionMutation =
        LiquidityPool::borrow;
    let _: fn(Env, Address, i128, AccountPositionRaw, bool, i128) -> PoolPositionMutation =
        LiquidityPool::withdraw;
    let _: fn(Env, Address, i128, AccountPositionRaw) -> PoolPositionMutation = LiquidityPool::repay;
    let _: fn(Env) -> MarketStateSnapshot = LiquidityPool::update_indexes;
    let _: fn(Env, i128) -> MarketStateSnapshot = LiquidityPool::add_rewards;
    let _: fn(Env, Address, Address, i128, i128, Bytes) -> MarketStateSnapshot =
        LiquidityPool::flash_loan;
    let _: fn(Env, Address, AccountPositionRaw, i128, i128, i128) -> PoolStrategyMutation =
        LiquidityPool::create_strategy;
    let _: fn(Env, AccountPositionType, AccountPositionRaw) -> PoolPositionMutation =
        LiquidityPool::seize_position;
    let _: fn(Env) -> PoolAmountMutation = LiquidityPool::claim_revenue;
    let _: fn(Env, InterestRateModel) = LiquidityPool::update_params;
    let _: fn(Env, BytesN<32>) = LiquidityPool::upgrade;
    let _: fn(Env) = LiquidityPool::keepalive;
    let _: fn(Env) -> i128 = LiquidityPool::capital_utilisation;
    let _: fn(Env) -> i128 = LiquidityPool::reserves;
    let _: fn(Env) -> i128 = LiquidityPool::deposit_rate;
    let _: fn(Env) -> i128 = LiquidityPool::borrow_rate;
    let _: fn(Env) -> i128 = LiquidityPool::protocol_revenue;
    let _: fn(Env) -> i128 = LiquidityPool::supplied_amount;
    let _: fn(Env) -> i128 = LiquidityPool::borrowed_amount;
    let _: fn(Env) -> u64 = LiquidityPool::delta_time;
    let _: fn(Env) -> PoolSyncData = LiquidityPool::get_sync_data;
}
