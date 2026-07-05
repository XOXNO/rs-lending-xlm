//! Pool calls exchange `ScaledPositionRaw` only.
//! The controller owns collateral risk parameters and merges them after pool mutations.

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolPositionMutation, PoolSeizeEntry,
    PoolStrategyMutation, PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Calls the pool to create a market for `hub_id`.
pub(crate) fn pool_create_market_call(
    env: &Env,
    pool_addr: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_market(&hub_id, params)
}

/// Calls the pool to apply supply entries, returning position mutations.
pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

/// Calls the pool to apply borrow entries for `receiver`, returning position mutations.
pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

/// Calls the pool to open a strategy position for `receiver`.
pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    fee: i128,
) -> PoolStrategyMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr)
        .create_strategy(receiver, &action, &fee)
}

/// Calls the pool to apply withdraw entries for `receiver`, returning position mutations.
pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).withdraw(
        receiver,
        &is_liquidation,
        entries,
    )
}

/// Calls the pool to apply repay actions for `payer`, returning position mutations.
pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

/// Calls the pool to seize the given liquidation positions.
pub(crate) fn pool_seize_positions_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).seize_positions(entries)
}

/// Calls the pool to execute a flash loan of `amount` to `receiver`.
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
    pool_interface::LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(hub_asset, initiator, receiver, &amount, &fee, data)
}

/// Calls the pool to accrue and update interest indexes for `hub_asset`.
pub(crate) fn pool_update_indexes_call(env: &Env, pool_addr: &Address, hub_asset: &HubAssetKey) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_indexes(hub_asset)
}

/// Calls the pool to claim accrued protocol revenue for `hub_asset`.
pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).claim_revenue(hub_asset)
}

/// Calls the pool to add `amount` of rewards to `hub_asset`.
pub(crate) fn pool_add_rewards_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).add_rewards(hub_asset, &amount)
}

/// Returns the pool's current sync data for `hub_asset`.
pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).get_sync_data(hub_asset)
}

/// Returns market indexes for the given hub-assets in one call.
pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).get_bulk_indexes(hub_assets)
}

/// Calls the pool to update the interest-rate model for `hub_asset`.
pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_params(hub_asset, params)
}

/// Calls the pool to upgrade to a new WASM hash.
pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}
