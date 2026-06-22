//! Liquidity pool ABI wrappers.
//!
//! Pool calls exchange `ScaledPositionRaw` only. Collateral risk parameters
//! remain controller-owned and are merged back after the pool returns scaled
//! supply or debt shares. Position verbs bundle their payload in `PoolAction`,
//! which carries the market asset the central pool routes on.

use controller_interface::types::{
    AccountPositionType, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolPositionMutation, PoolStrategyMutation,
    PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

pub(crate) fn pool_create_market_call(env: &Env, pool_addr: &Address, params: &MarketParamsRaw) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_market(params)
}

pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    fee: i128,
) -> PoolStrategyMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).create_strategy(receiver, &action, &fee)
}

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

pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

pub(crate) fn pool_seize_position_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).seize_position(asset, &side, &position)
}

pub(crate) fn pool_flash_loan_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr)
        .flash_loan(asset, initiator, receiver, &amount, &fee, data)
}

pub(crate) fn pool_update_indexes_call(env: &Env, pool_addr: &Address, asset: &Address) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_indexes(asset)
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
) -> PoolAmountMutation {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).claim_revenue(asset)
}

pub(crate) fn pool_add_rewards_call(env: &Env, pool_addr: &Address, asset: &Address, amount: i128) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).add_rewards(asset, &amount)
}

pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
) -> PoolSyncData {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).get_sync_data(asset)
}

pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    assets: &Vec<Address>,
) -> Vec<MarketIndexRaw> {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).bulk_get_indexes(assets)
}

pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    params: &InterestRateModel,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_params(asset, params)
}

pub(crate) fn pool_update_caps_call(
    env: &Env,
    pool_addr: &Address,
    asset: &Address,
    supply_cap: i128,
    borrow_cap: i128,
) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).update_caps(asset, &supply_cap, &borrow_cap)
}

pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    pool_interface::LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}
