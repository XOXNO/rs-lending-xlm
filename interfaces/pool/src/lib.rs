#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountPositionType, InterestRateModel, MarketIndexRaw, MarketParamsRaw, MarketStateSnapshot,
    PoolAction, PoolAmountMutation, PoolBorrowEntry, PoolPositionMutation, PoolStrategyMutation,
    PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env, Vec};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    /// Creates an asset market with fresh RAY indexes.
    fn create_market(env: Env, params: MarketParamsRaw);

    /// Supplies each entry and returns the updated scaled shares, input-
    /// ordered. No counterparty: the controller pre-transfers the tokens.
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>;

    /// Borrows each entry, transferring tokens to `receiver`; input-ordered.
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Withdraws each entry (full position at the i128::MAX sentinel) to
    /// `receiver`; `is_liquidation` applies to the whole call; input-ordered.
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Repays each action (tokens pre-transferred by the controller),
    /// refunding overpayments to `payer`; input-ordered.
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>;
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

    /// Creates strategy debt and transfers `action.amount - fee` to `receiver`.
    fn create_strategy(
        env: Env,
        receiver: Address,
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
    /// Borrow/supply indexes accrued to the current ledger time for each
    /// asset, index-aligned with the request. One call replaces N
    /// `get_sync_data` reads for flows that only need indexes.
    fn bulk_get_sync_data(env: Env, assets: Vec<Address>) -> Vec<MarketIndexRaw>;
}
