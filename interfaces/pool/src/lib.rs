#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPositionRaw, AccountPositionType, InterestRateModel, MarketStateSnapshot,
    PoolAmountMutation, PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    fn supply(
        env: Env,
        position: AccountPositionRaw,
        amount: i128,
        supply_cap: i128,
    ) -> PoolPositionMutation;
    fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPositionRaw,
        borrow_cap: i128,
    ) -> PoolPositionMutation;
    fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPositionRaw,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation;
    fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPositionRaw,
    ) -> PoolPositionMutation;
    fn update_indexes(env: Env) -> MarketStateSnapshot;
    fn add_rewards(env: Env, amount: i128) -> MarketStateSnapshot;
    fn flash_loan(
        env: Env,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot;
    fn create_strategy(
        env: Env,
        caller: Address,
        position: AccountPositionRaw,
        amount: i128,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation;
    fn seize_position(
        env: Env,
        side: AccountPositionType,
        position: AccountPositionRaw,
    ) -> PoolPositionMutation;
    fn claim_revenue(env: Env) -> PoolAmountMutation;
    fn update_params(env: Env, model: InterestRateModel);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn keepalive(env: Env);
    fn capital_utilisation(env: Env) -> i128;
    fn reserves(env: Env) -> i128;
    fn deposit_rate(env: Env) -> i128;
    fn borrow_rate(env: Env) -> i128;
    fn protocol_revenue(env: Env) -> i128;
    fn supplied_amount(env: Env) -> i128;
    fn borrowed_amount(env: Env) -> i128;
    fn delta_time(env: Env) -> u64;
    fn get_sync_data(env: Env) -> PoolSyncData;
}
