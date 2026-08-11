//! Thin wrapper functions around `LiquidityPoolClient`, the generated client
//! for the spoke lending pool contract. Each function forwards its arguments
//! to a single cross-contract call on the pool at `pool_addr` and returns
//! whatever that call returns.

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Calls `create_market` on the pool to initialize a market for `hub_id` with
/// the given interest-rate and risk parameters.
pub(crate) fn pool_create_market_call(
    env: &Env,
    pool_addr: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) {
    LiquidityPoolClient::new(env, pool_addr).create_market(&hub_id, params)
}

/// Calls `supply` on the pool with `entries`, returning the resulting
/// position mutations.
pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

/// Calls `borrow` on the pool for `receiver` with `entries`, returning the
/// resulting position mutations.
pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

/// Calls `create_strategy` on the pool for `receiver` with the given
/// `action`, returning the resulting strategy mutation. `charge_fee`
/// controls whether the pool applies its fee for this call.
pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    LiquidityPoolClient::new(env, pool_addr).create_strategy(receiver, &action, &charge_fee)
}

/// Calls `withdraw` on the pool for `receiver` with `entries`, returning the
/// resulting position mutations. `is_liquidation` tells the pool whether the
/// withdrawal is part of a liquidation.
pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).withdraw(receiver, &is_liquidation, entries)
}

/// Calls `repay` on the pool for `payer` with `actions`, returning the
/// resulting position mutations.
pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

/// Calls `net_settle` on the pool with `entry`, returning the settlement
/// result.
pub(crate) fn pool_net_settle_call(
    env: &Env,
    pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    LiquidityPoolClient::new(env, pool_addr).net_settle(entry)
}

/// Calls `seize_positions` on the pool with `entries`.
pub(crate) fn pool_seize_positions_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    LiquidityPoolClient::new(env, pool_addr).seize_positions(entries)
}

/// Calls `flash_loan` on the pool for `hub_asset`, lending `amount` to
/// `receiver` on behalf of `initiator` with callback `data`.
pub(crate) fn pool_flash_loan_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    data: &Bytes,
) -> i128 {
    LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(hub_asset, initiator, receiver, &amount, data)
}

/// Calls `update_indexes` on the pool for `hub_asset`.
pub(crate) fn pool_update_indexes_call(env: &Env, pool_addr: &Address, hub_asset: &HubAssetKey) {
    LiquidityPoolClient::new(env, pool_addr).update_indexes(hub_asset)
}

/// Calls `claim_revenue` on the pool for `hub_asset`, returning the resulting
/// amount mutation.
pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).claim_revenue(hub_asset)
}

/// Calls `recapitalize` on the pool for `hub_asset`, drawing `amount` from
/// `payer`. Returns the resulting amount mutation.
pub(crate) fn pool_recapitalize_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    payer: &Address,
    amount: i128,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).recapitalize(hub_asset, payer, &amount)
}

/// Calls `get_sync_data` on the pool for `hub_asset`, returning the pool's
/// sync data for that market.
pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    LiquidityPoolClient::new(env, pool_addr).get_sync_data(hub_asset)
}

/// Calls `get_bulk_indexes` on the pool for `hub_assets`, returning the raw
/// market indexes for each requested asset.
pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    LiquidityPoolClient::new(env, pool_addr).get_bulk_indexes(hub_assets)
}

/// Calls `update_params` on the pool for `hub_asset`, replacing its
/// interest-rate model with `params`.
pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    LiquidityPoolClient::new(env, pool_addr).update_params(hub_asset, params)
}

/// Calls `upgrade` on the pool, pointing it at `new_wasm_hash`.
pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}
