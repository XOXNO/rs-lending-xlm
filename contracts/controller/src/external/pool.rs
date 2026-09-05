use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Creates a hub market with the supplied asset and interest-rate parameters.
pub(crate) fn pool_create_market_call(
    env: &Env,
    pool_addr: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) {
    LiquidityPoolClient::new(env, pool_addr).create_market(&hub_id, params)
}

/// Credits prefunded deposits and returns updated scaled positions and indexes.
/// The controller must transfer and measure receipts before this call.
pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

/// Mints scaled debt and transfers borrowed assets to `receiver`.
pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

/// Mints debt for `action.amount` and transfers it to `receiver`, less the
/// strategy fee when `charge_fee` is set.
pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    LiquidityPoolClient::new(env, pool_addr).create_strategy(receiver, &action, &charge_fee)
}

/// Burns supply and sends underlying to `receiver`. Liquidation skips utilization
/// caps and may withhold protocol fees; returned amounts are gross of fees.
pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).withdraw(receiver, &is_liquidation, entries)
}

/// Burns debt against prefunded payments and refunds overpayment to `payer`.
pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

/// Nets supply against debt within one market, burning matched scaled amounts
/// without moving cash.
pub(crate) fn pool_net_settle_call(
    env: &Env,
    pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    LiquidityPoolClient::new(env, pool_addr).net_settle(entry)
}

/// Seizes positions: debt losses reduce the supply index; seized supply shares
/// become protocol revenue.
pub(crate) fn pool_seize_positions_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    LiquidityPoolClient::new(env, pool_addr).seize_positions(entries)
}

/// Lends to `receiver`, invokes its callback, and collects principal plus fee.
/// Returns the fee charged.
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

/// Accrues and persists market indexes through the current ledger time.
pub(crate) fn pool_update_indexes_call(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) {
    LiquidityPoolClient::new(env, pool_addr).update_indexes(hub_assets)
}

/// Claims accrued revenue to the pool owner (the controller). Returns the pool
/// mutation; callers must measure the controller's actual receipt.
pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).claim_revenue(hub_asset)
}

/// Applies prefunded cash up to the backing shortfall and refunds excess to
/// `payer`. Returns the amount credited; does not pull funds from `payer`.
pub(crate) fn pool_recapitalize_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    payer: &Address,
    amount: i128,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).recapitalize(hub_asset, payer, &amount)
}

/// Reads stored market parameters and state without accruing interest.
pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    LiquidityPoolClient::new(env, pool_addr).get_sync_data(hub_asset)
}

/// Returns simulated current indexes in request order without writing state.
pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    LiquidityPoolClient::new(env, pool_addr).get_bulk_indexes(hub_assets)
}

/// Accrues interest, then replaces the rate model and flash-loan settings.
pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    LiquidityPoolClient::new(env, pool_addr).update_params(hub_asset, params)
}

/// Upgrades the pool Wasm to `new_wasm_hash`.
pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}

#[cfg(test)]
#[path = "../../tests/external/pool.rs"]
mod tests;
