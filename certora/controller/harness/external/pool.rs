//! Certora harness for `controller::external::pool`.
//! Production ABI; each entry summarized independently via shared pool summaries.
//! Bulk returns are length-preserving and input-ordered.

use crate::spec::summaries::bulk_index_summary;
use crate::spec::summaries::pool::{
    add_rewards_summary, borrow_summary, claim_revenue_summary, create_strategy_summary,
    flash_loan_summary, get_sync_data_summary, net_settle_summary, repay_summary,
    seize_positions_summary, supply_summary, update_indexes_summary, withdraw_summary,
};
use crate::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// No-op privileged-config call (resolves production import under certora).
pub(crate) fn pool_create_market_call(
    _env: &Env,
    _pool_addr: &Address,
    _hub_id: u32,
    _params: &MarketParamsRaw,
) {
}

pub(crate) fn pool_supply_call(
    env: &Env,
    _pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for entry in entries.iter() {
        out.push_back(supply_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.position.clone(),
            entry.action.amount,
        ));
    }
    out
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    _pool_addr: &Address,
    _receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for entry in entries.iter() {
        out.push_back(borrow_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.amount,
            entry.action.position.clone(),
        ));
    }
    out
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    _pool_addr: &Address,
    _receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    create_strategy_summary(
        env,
        &action.hub_asset.asset,
        action.position,
        action.amount,
        charge_fee,
    )
}

pub(crate) fn pool_withdraw_call(
    env: &Env,
    _pool_addr: &Address,
    _receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for entry in entries.iter() {
        out.push_back(withdraw_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.amount,
            entry.action.position.clone(),
            is_liquidation,
            entry.protocol_fee,
        ));
    }
    out
}

pub(crate) fn pool_repay_call(
    env: &Env,
    _pool_addr: &Address,
    _payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for action in actions.iter() {
        out.push_back(repay_summary(
            env,
            &action.hub_asset.asset,
            action.amount,
            action.position.clone(),
        ));
    }
    out
}

pub(crate) fn pool_net_settle_call(
    env: &Env,
    _pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    net_settle_summary(
        env,
        &entry.hub_asset.asset,
        entry.amount,
        entry.supply_position.clone(),
        entry.debt_position.clone(),
    )
}

pub(crate) fn pool_seize_positions_call(
    env: &Env,
    _pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    seize_positions_summary(env, entries)
}

pub(crate) fn pool_flash_loan_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    data: &Bytes,
) -> i128 {
    flash_loan_summary(env, &hub_asset.asset, initiator, receiver, amount, data)
}

pub(crate) fn pool_update_indexes_call(env: &Env, _pool_addr: &Address, hub_asset: &HubAssetKey) {
    update_indexes_summary(env, &hub_asset.asset)
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    claim_revenue_summary(env, &hub_asset.asset)
}

pub(crate) fn pool_add_rewards_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
) {
    add_rewards_summary(env, &hub_asset.asset, amount)
}

pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    get_sync_data_summary(env, &hub_asset.asset)
}

// Index-cache miss: nondet indexes bounded by production floors.
pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    _pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    let mut out: Vec<MarketIndexRaw> = Vec::new(env);
    for hub_asset in hub_assets.iter() {
        out.push_back(bulk_index_summary(env, &hub_asset.asset));
    }
    out
}

// No-op privileged-config calls (resolve production imports under certora).

pub(crate) fn pool_update_params_call(
    _env: &Env,
    _pool_addr: &Address,
    _hub_asset: &HubAssetKey,
    _params: &InterestRateModel,
) {
}

pub(crate) fn pool_upgrade_call(_env: &Env, _pool_addr: &Address, _new_wasm_hash: &BytesN<32>) {}
