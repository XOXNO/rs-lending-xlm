//! Pool calls exchange `ScaledPositionRaw` only.
//! The controller owns collateral risk parameters and merges them after pool mutations.

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

pub(crate) fn pool_create_market_call(
    env: &Env,
    pool_addr: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) {
    LiquidityPoolClient::new(env, pool_addr).create_market(&hub_id, params)
}

pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    LiquidityPoolClient::new(env, pool_addr).create_strategy(receiver, &action, &charge_fee)
}

pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).withdraw(receiver, &is_liquidation, entries)
}

pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

/// Nets a supply leg against a debt leg on the same hub-asset with zero token transfer.
pub(crate) fn pool_net_settle_call(
    env: &Env,
    pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    LiquidityPoolClient::new(env, pool_addr).net_settle(entry)
}

pub(crate) fn pool_seize_positions_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    LiquidityPoolClient::new(env, pool_addr).seize_positions(entries)
}

pub(crate) fn pool_flash_loan_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) {
    LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(hub_asset, initiator, receiver, &amount, &fee, data)
}

pub(crate) fn pool_update_indexes_call(env: &Env, pool_addr: &Address, hub_asset: &HubAssetKey) {
    LiquidityPoolClient::new(env, pool_addr).update_indexes(hub_asset)
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).claim_revenue(hub_asset)
}

pub(crate) fn pool_add_rewards_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
) {
    LiquidityPoolClient::new(env, pool_addr).add_rewards(hub_asset, &amount)
}

pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    LiquidityPoolClient::new(env, pool_addr).get_sync_data(hub_asset)
}

pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    LiquidityPoolClient::new(env, pool_addr).get_bulk_indexes(hub_assets)
}

pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    LiquidityPoolClient::new(env, pool_addr).update_params(hub_asset, params)
}

pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}
