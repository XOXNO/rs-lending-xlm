#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPositionType, InterestRateModel, MarketParamsRaw, MarketStateSnapshot, PoolAction,
    PoolAmountMutation, PoolPositionMutation, PoolStrategyMutation, PoolSyncData,
    ScaledPositionRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    /// Creates an asset market with fresh RAY indexes.
    fn create_market(env: Env, params: MarketParamsRaw);

    /// Supplies an amount and returns the updated scaled share.
    fn supply(env: Env, action: PoolAction, supply_cap: i128) -> PoolPositionMutation;

    /// Borrows an amount and returns the updated scaled debt share.
    fn borrow(env: Env, action: PoolAction, borrow_cap: i128) -> PoolPositionMutation;

    /// Withdraws up to `action.amount`, or full position at `i128::MAX`.
    fn withdraw(
        env: Env,
        action: PoolAction,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation;

    /// Repays debt and returns the updated scaled debt share.
    fn repay(env: Env, action: PoolAction) -> PoolPositionMutation;
    fn update_indexes(env: Env, asset: Address) -> MarketStateSnapshot;
    fn add_rewards(env: Env, asset: Address, amount: i128) -> MarketStateSnapshot;
    /// Executes a flash loan that must return `amount + fee`.
    fn flash_loan(
        env: Env,
        asset: Address,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot;

    /// Creates strategy debt and transfers `action.amount - fee`.
    fn create_strategy(
        env: Env,
        action: PoolAction,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation;

    /// Removes a seized liquidation or bad-debt position.
    fn seize_position(
        env: Env,
        asset: Address,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;

    /// Claims protocol revenue capped by reserves and claimable shares.
    fn claim_revenue(env: Env, asset: Address) -> PoolAmountMutation;
    fn update_params(env: Env, asset: Address, model: InterestRateModel);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn capital_utilisation(env: Env, asset: Address) -> i128;
    fn reserves(env: Env, asset: Address) -> i128;
    fn deposit_rate(env: Env, asset: Address) -> i128;
    fn borrow_rate(env: Env, asset: Address) -> i128;
    fn protocol_revenue(env: Env, asset: Address) -> i128;
    fn supplied_amount(env: Env, asset: Address) -> i128;
    fn borrowed_amount(env: Env, asset: Address) -> i128;
    fn delta_time(env: Env, asset: Address) -> u64;
    fn get_sync_data(env: Env, asset: Address) -> PoolSyncData;
}
