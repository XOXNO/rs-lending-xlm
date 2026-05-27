#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPositionType, InterestRateModel, MarketStateSnapshot, PoolAmountMutation,
    PoolPositionMutation, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    /// Supply `amount` of the pool's asset. Returns the new scaled position
    /// and a snapshot of the pool state after interest accrual and cap check.
    fn supply(
        env: Env,
        position: ScaledPositionRaw,
        amount: i128,
        supply_cap: i128,
    ) -> PoolPositionMutation;

    /// Borrow `amount`. Caller must have authorized the controller.
    /// Accrues interest, checks reserves + utilization cap, returns scaled debt.
    fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
        borrow_cap: i128,
    ) -> PoolPositionMutation;

    /// Withdraw up to `amount` (or all if i128::MAX). `is_liquidation` changes
    /// fee handling. Returns the gross amount withdrawn before protocol fee.
    fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation;

    /// Repay debt. Any overpayment is refunded to the caller immediately.
    fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;
    fn update_indexes(env: Env) -> MarketStateSnapshot;
    fn add_rewards(env: Env, amount: i128) -> MarketStateSnapshot;
    /// Execute a flash loan. The receiver must repay amount+fee in the same tx.
    /// Fee is added to protocol revenue.
    fn flash_loan(
        env: Env,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot;

    /// Strategy entry (used by controller for multiply/swap etc.). Borrows
    /// `amount`, sends `amount - fee` to caller, records fee as revenue.
    fn create_strategy(
        env: Env,
        caller: Address,
        position: ScaledPositionRaw,
        amount: i128,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation;

    /// Seize a fully written-down position (liquidation or bad-debt cleanup).
    /// For borrows: socializes the debt by reducing the supply index.
    /// For deposits: absorbs remaining dust into revenue.
    fn seize_position(
        env: Env,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;

    /// Claim accumulated protocol revenue (owner only). Transfers the lesser
    /// of on-chain reserves and claimable revenue.
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
