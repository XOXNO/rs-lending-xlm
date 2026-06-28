//! Certora harness substitute for `controller::external::pool`.
//!
//! Under `--features certora --no-default-features`,
//! `controller/src/external/mod.rs` path-swaps the production `pool` module to
//! this file. Each wrapper mirrors the production ABI in
//! `contracts/controller/src/external/pool.rs` exactly, but instead of issuing a
//! cross-contract `LiquidityPoolClient` call it returns the bounded nondet model
//! from `certora/shared/summaries/pool.rs`.
//!
//! The central-pool ABI bulks the position verbs into `Vec<entry>` and returns
//! `Vec<PoolPositionMutation>`. The harness models the batch element-wise:
//! each entry is summarized independently and pushed input-ordered, so the
//! returned `Vec` is length-preserving and every per-entry postcondition holds
//! at its own index (matching the `results.get(i)` reads in production).

use crate::spec::summaries::bulk_index_summary;
use crate::spec::summaries::pool::{
    add_rewards_summary, borrow_summary, claim_revenue_summary, create_strategy_summary,
    flash_loan_summary, get_sync_data_summary, repay_summary, seize_position_summary,
    supply_summary, update_indexes_summary, withdraw_summary,
};
use crate::types::{
    AccountPositionType, HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw,
    PoolAction, PoolAmountMutation, PoolBorrowEntry, PoolPositionMutation, PoolStrategyMutation,
    PoolSupplyEntry, PoolSyncData, PoolWithdrawEntry, ScaledPositionRaw,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Void privileged-config call. No return value to summarize, so the prover
/// treats it as a no-op. Exists only so the production import in `router.rs`
/// resolves under the certora feature.
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
            0,
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
            0,
        ));
    }
    out
}

pub(crate) fn pool_create_strategy_call(
    env: &Env,
    _pool_addr: &Address,
    _receiver: &Address,
    action: PoolAction,
    fee: i128,
) -> PoolStrategyMutation {
    create_strategy_summary(
        env,
        &action.hub_asset.asset,
        action.position,
        action.amount,
        fee,
        0,
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

pub(crate) fn pool_seize_position_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
    side: AccountPositionType,
    position: ScaledPositionRaw,
) -> PoolPositionMutation {
    seize_position_summary(env, &hub_asset.asset, side, position)
}

pub(crate) fn pool_flash_loan_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
    initiator: &Address,
    receiver: &Address,
    amount: i128,
    fee: i128,
    data: &Bytes,
) {
    flash_loan_summary(env, &hub_asset.asset, initiator, receiver, amount, fee, data)
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

// Backs the controller's index cache on a miss (`cache::Cache::cached_market_index`).
// Each hub-asset gets a nondet index bounded by production floors.
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

// Void privileged-config calls have no return value to summarize, so the
// prover treats them as no-ops. They exist only so the production import path
// in `router.rs` resolves under the certora feature.

pub(crate) fn pool_update_params_call(
    _env: &Env,
    _pool_addr: &Address,
    _hub_asset: &HubAssetKey,
    _params: &InterestRateModel,
) {
}

pub(crate) fn pool_update_caps_call(
    _env: &Env,
    _pool_addr: &Address,
    _hub_asset: &HubAssetKey,
    _supply_cap: i128,
    _borrow_cap: i128,
) {
}

pub(crate) fn pool_upgrade_call(_env: &Env, _pool_addr: &Address, _new_wasm_hash: &BytesN<32>) {}
