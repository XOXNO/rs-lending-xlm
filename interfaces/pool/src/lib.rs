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
    /// Registers a new asset market keyed by `params.asset_id`.
    ///
    /// Panics AssetAlreadySupported when the market already exists; writes the
    /// validated params and a fresh state (indexes at RAY) to persistent storage.
    fn create_market(env: Env, params: MarketParamsRaw);

    /// Supplies `action.amount` of `action.asset` and returns the updated
    /// scaled share.
    ///
    /// Interest accrues first and the underlying post-supply total must remain
    /// within `supply_cap`. `action.caller` is carried but unused: the
    /// controller transfers the tokens in before this call.
    fn supply(env: Env, action: PoolAction, supply_cap: i128) -> PoolPositionMutation;

    /// Borrows `action.amount` of `action.asset` to `action.caller` and
    /// returns the updated scaled debt share.
    ///
    /// Interest accrues first; reserves, borrow cap, and max utilization are
    /// checked before the token transfer.
    fn borrow(env: Env, action: PoolAction, borrow_cap: i128) -> PoolPositionMutation;

    /// Withdraws up to `action.amount`, or the full position when
    /// `action.amount == i128::MAX`.
    ///
    /// Liquidation calls may deduct `protocol_fee`; `actual_amount` remains the
    /// gross withdrawn amount before that fee.
    fn withdraw(
        env: Env,
        action: PoolAction,
        is_liquidation: bool,
        protocol_fee: i128,
    ) -> PoolPositionMutation;

    /// Repays debt on `action.asset` and returns the updated scaled debt share.
    ///
    /// Any amount above the ceiling-rounded current debt is refunded to
    /// `action.caller`.
    fn repay(env: Env, action: PoolAction) -> PoolPositionMutation;
    fn update_indexes(env: Env, asset: Address) -> MarketStateSnapshot;
    fn add_rewards(env: Env, asset: Address, amount: i128) -> MarketStateSnapshot;
    /// Executes a flash loan of `asset` that must be repaid with `amount + fee`.
    ///
    /// The fee is recorded as protocol revenue after the pool balance check.
    fn flash_loan(
        env: Env,
        asset: Address,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    ) -> MarketStateSnapshot;

    /// Creates strategy debt on `action.asset` and sends `action.amount - fee`
    /// to `action.caller`.
    ///
    /// The fee is recorded as protocol revenue and `amount_received` is the net
    /// asset amount made available to the strategy.
    fn create_strategy(
        env: Env,
        action: PoolAction,
        fee: i128,
        borrow_cap: i128,
    ) -> PoolStrategyMutation;

    /// Removes a fully seized position during liquidation or bad-debt cleanup.
    ///
    /// Borrow seizures reduce the supply index to socialize bad debt; deposit
    /// seizures move residual scaled supply into revenue.
    fn seize_position(
        env: Env,
        asset: Address,
        side: AccountPositionType,
        position: ScaledPositionRaw,
    ) -> PoolPositionMutation;

    /// Claims protocol revenue for `asset`, capped by live reserves and
    /// claimable shares.
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
