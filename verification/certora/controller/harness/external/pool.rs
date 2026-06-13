//! Certora harness substitute for `controller::external::pool`.
//!
//! Under `--features certora`, `controller/src/external/mod.rs` path-swaps
//! this file in. Each production wrapper is adapted onto the bounded nondet
//! summaries in `verification/certora/shared/summaries/pool.rs`; the bulk
//! endpoints map every entry through its per-entry summary and return the
//! mutations input-ordered, mirroring the pool's loop semantics.

use common::types::{
    AccountPositionType, InterestRateModel, MarketParamsRaw, MarketStateSnapshot, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolPositionMutation, PoolStrategyMutation,
    PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

use crate::spec::summaries::pool::{
    add_rewards_summary, borrow_summary, claim_revenue_summary, create_strategy_summary,
    flash_loan_summary, get_sync_data_summary, repay_summary, seize_position_summary,
    supply_summary, update_indexes_summary, withdraw_summary,
};

pub(crate) fn pool_create_market_call(_env: &Env, _pool_addr: &Address, _params: &MarketParamsRaw) {
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
            &entry.action.asset,
            entry.action.position.clone(),
            entry.action.amount,
            entry.supply_cap,
        ));
    }
    out
}

pub(crate) fn pool_borrow_call(
    env: &Env,
    _pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for entry in entries.iter() {
        out.push_back(borrow_summary(
            env,
            &entry.action.asset,
            receiver.clone(),
            entry.action.amount,
            entry.action.position.clone(),
            entry.borrow_cap,
        ));
    }
    out
}

pub(crate) fn pool_withdraw_call(
    env: &Env,
    _pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for entry in entries.iter() {
        out.push_back(withdraw_summary(
            env,
            &entry.action.asset,
            receiver.clone(),
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
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    let mut out: Vec<PoolPositionMutation> = Vec::new(env);
    for action in actions.iter() {
        out.push_back(repay_summary(
            env,
            &action.asset,
            payer.clone(),
            action.amount,
            action.position.clone(),
        ));
    }
    out
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    _pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    fee: i128,
    borrow_cap: i128,
) -> PoolStrategyMutation {
    create_strategy_summary(
        env,
        &action.asset,
        receiver.clone(),
        action.position,
        action.amount,
        fee,
        borrow_cap,
    )
}

pub(crate) fn pool_seize_position_call(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
    side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    seize_position_summary(env, asset, side, position)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pool_flash_loan_call(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) -> MarketStateSnapshot {
    flash_loan_summary(env, asset, initiator, receiver, amount, fee, data)
}

pub(crate) fn pool_update_indexes_call(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
) -> MarketStateSnapshot {
    update_indexes_summary(env, asset)
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
) -> PoolAmountMutation {
    claim_revenue_summary(env, asset)
}

pub(crate) fn pool_add_rewards_call(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
    _amount: i128,
) -> MarketStateSnapshot {
    add_rewards_summary(env, asset, _amount)
}

pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    _pool_addr: &Address,
    asset: &Address,
) -> PoolSyncData {
    get_sync_data_summary(env, asset)
}

pub(crate) fn pool_update_params_call(
    _env: &Env,
    _pool_addr: &Address,
    _asset: &Address,
    _params: &InterestRateModel,
) {
}

pub(crate) fn pool_upgrade_call(_env: &Env, _pool_addr: &Address, _new_wasm_hash: &BytesN<32>) {}
