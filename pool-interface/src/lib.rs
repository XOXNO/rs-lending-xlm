#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPosition, MarketIndex, PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
};
use soroban_sdk::{contractclient, Address, BytesN, Env};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    fn supply(
        env: Env,
        position: AccountPosition,
        price_wad: i128,
        amount: i128,
    ) -> PoolPositionMutation;
    fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation;
    fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        is_liquidation: bool,
        protocol_fee: i128,
        price_wad: i128,
    ) -> PoolPositionMutation;
    /// H-02: parameter order aligned with `borrow` so the two i128 args
    /// (`amount`, `price_wad`) sit in identical positions across both
    /// endpoints. A controller-side typo can no longer silently swap them.
    fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        position: AccountPosition,
        price_wad: i128,
    ) -> PoolPositionMutation;
    fn update_indexes(env: Env, price_wad: i128) -> MarketIndex;
    fn add_rewards(env: Env, price_wad: i128, amount: i128);
    /// Pool always uses its own `params.asset_id` for the token transfer;
    /// no caller-supplied `asset` argument (audit fix H-01).
    fn flash_loan_begin(env: Env, amount: i128, receiver: Address);
    fn flash_loan_end(env: Env, amount: i128, fee: i128, receiver: Address);
    fn create_strategy(
        env: Env,
        caller: Address,
        position: AccountPosition,
        amount: i128,
        fee: i128,
        price_wad: i128,
    ) -> PoolStrategyMutation;
    fn seize_position(env: Env, position: AccountPosition, price_wad: i128) -> AccountPosition;
    /// L-05: pool transfers revenue to the accumulator address it stored
    /// at construction; no caller-supplied destination. Closes the
    /// controller-misconfig surface where revenue could be routed
    /// out-of-band.
    fn claim_revenue(env: Env, price_wad: i128) -> i128;
    fn update_params(
        env: Env,
        max_borrow_rate: i128,
        base_borrow_rate: i128,
        slope1: i128,
        slope2: i128,
        slope3: i128,
        mid_utilization: i128,
        optimal_utilization: i128,
        reserve_factor: i128,
    );
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
