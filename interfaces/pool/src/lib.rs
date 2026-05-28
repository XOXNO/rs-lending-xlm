#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPositionType, InterestRateModel, MarketStateSnapshot, PoolAmountMutation,
    PoolPositionMutation, PoolStrategyMutation, PoolSyncData, ScaledPositionRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    /// Supplies `amount` of the pool asset and returns the updated scaled share.
    ///
    /// Interest accrues first and the underlying post-supply total must remain
    /// within `supply_cap`.
    fn supply(
        env: Env,
        position: ScaledPositionRaw,
        amount: i128,
        supply_cap: i128,
    ) -> PoolPositionMutation;

    /// Borrows `amount` to `caller` and returns the updated scaled debt share.
    ///
    /// Interest accrues first; reserves, borrow cap, and max utilization are
    /// checked before the token transfer.
    fn borrow(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
        borrow_cap: i128,
    ) -> PoolPositionMutation;

    /// Withdraws up to `amount`, or the full position when `amount == i128::MAX`.
    ///
    /// Liquidation calls may deduct `protocol_fee`; `actual_amount` remains the
    /// gross withdrawn amount before that fee.
    fn withdraw(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation;

    /// Repays debt and returns the updated scaled debt share.
    ///
    /// Any amount above the ceiling-rounded current debt is refunded to `caller`.
    fn repay(
        env: Env,
        caller: Address,
        amount: i128,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;
    fn update_indexes(env: Env) -> MarketStateSnapshot;
    fn add_rewards(env: Env, amount: i128) -> MarketStateSnapshot;
    /// Executes a flash loan that must be repaid with `amount + fee`.
    ///
    /// The fee is recorded as protocol revenue after the pool balance check.
    fn flash_loan(
        env: Env,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot;

    /// Creates strategy debt and sends `amount - fee` to `caller`.
    ///
    /// The fee is recorded as protocol revenue and `amount_received` is the net
    /// asset amount made available to the strategy.
    fn create_strategy(
        env: Env,
        caller: Address,
        position: ScaledPositionRaw,
        amount: i128,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation;

    /// Removes a fully seized position during liquidation or bad-debt cleanup.
    ///
    /// Borrow seizures reduce the supply index to socialize bad debt; deposit
    /// seizures move residual scaled supply into revenue.
    fn seize_position(
        env: Env,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;

    /// Claims protocol revenue, capped by live reserves and claimable shares.
    fn claim_revenue(env: Env) -> PoolAmountMutation;
    fn update_params(env: Env, model: InterestRateModel);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
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
