use crate::spec::ghost_prices;
use crate::spec::summaries::pool::{
    borrow_summary, claim_revenue_summary, create_strategy_summary, flash_loan_summary,
    net_settle_summary, recapitalize_summary, repay_summary, seize_positions_summary,
    supply_summary, update_indexes_summary, withdraw_summary,
};
use crate::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Replaces a mutation's index pair with the market's snapshot for this rule.
///
/// The pool returns the market's index *after* accrual, and `get_bulk_indexes`
/// and `get_sync_data` return the same accrued state, so within one
/// transaction a market has exactly one index pair no matter which door it is
/// read through. The per-verb summaries draw independently, and the controller
/// then writes the drawn value into the cache with `Context::put_market_index`,
/// which is what the post-pool risk gate reads. Without this, a rule that
/// values the same position after the call reads a *different* index from the
/// one the gate used, and no composition rule can hold.
fn snapshot_index(env: &Env, hub_asset: &HubAssetKey) -> MarketIndexRaw {
    ghost_prices::market_index(env, hub_asset)
}

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
        let mut mutation = supply_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.position.clone(),
            entry.action.amount,
        );
        mutation.market_index = snapshot_index(env, &entry.action.hub_asset);
        out.push_back(mutation);
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
        let mut mutation = borrow_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.amount,
            entry.action.position.clone(),
        );
        mutation.market_index = snapshot_index(env, &entry.action.hub_asset);
        out.push_back(mutation);
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
    let mut mutation = create_strategy_summary(
        env,
        &action.hub_asset.asset,
        action.position,
        action.amount,
        charge_fee,
    );
    mutation.market_index = snapshot_index(env, &action.hub_asset);
    mutation
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
        let mut mutation = withdraw_summary(
            env,
            &entry.action.hub_asset.asset,
            entry.action.amount,
            entry.action.position.clone(),
            is_liquidation,
            entry.protocol_fee,
        );
        mutation.market_index = snapshot_index(env, &entry.action.hub_asset);
        out.push_back(mutation);
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
        let mut mutation = repay_summary(
            env,
            &action.hub_asset.asset,
            action.amount,
            action.position.clone(),
        );
        mutation.market_index = snapshot_index(env, &action.hub_asset);
        out.push_back(mutation);
    }
    out
}

pub(crate) fn pool_net_settle_call(
    env: &Env,
    _pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    let mut result = net_settle_summary(
        env,
        &entry.hub_asset.asset,
        entry.amount,
        entry.supply_position.clone(),
        entry.debt_position.clone(),
    );
    result.market_index = snapshot_index(env, &entry.hub_asset);
    result
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

pub(crate) fn pool_update_indexes_call(
    env: &Env,
    _pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) {
    for hub_asset in hub_assets.iter() {
        update_indexes_summary(env, &hub_asset.asset);
    }
}

pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    claim_revenue_summary(env, &hub_asset.asset)
}

pub(crate) fn pool_recapitalize_call(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
    _payer: &Address,
    amount: i128,
) -> PoolAmountMutation {
    recapitalize_summary(env, &hub_asset.asset, amount)
}

/// One market snapshot per rule: repeated sync reads of the same market, and
/// the bulk index read below, replay the first draw instead of drawing again.
pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    _pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    ghost_prices::sync_data(env, hub_asset)
}

pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    _pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    let mut out: Vec<MarketIndexRaw> = Vec::new(env);
    for hub_asset in hub_assets.iter() {
        out.push_back(ghost_prices::market_index(env, &hub_asset));
    }
    out
}

pub(crate) fn pool_update_params_call(
    _env: &Env,
    _pool_addr: &Address,
    _hub_asset: &HubAssetKey,
    _params: &InterestRateModel,
) {
}

pub(crate) fn pool_upgrade_call(_env: &Env, _pool_addr: &Address, _new_wasm_hash: &BytesN<32>) {}
