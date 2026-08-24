#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env, Vec};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw);

    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel);

    fn update_indexes(env: Env, hub_assets: Vec<HubAssetKey>);

    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>;

    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation>;

    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation>;

    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>;

    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult;

    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>);

    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        data: Bytes,
    ) -> i128;

    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation;

    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation;

    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation;

    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);

    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128;

    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64;

    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData;

    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>;
}
