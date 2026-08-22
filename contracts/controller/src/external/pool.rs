use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

/// Creates a new market for `hub_id` on the pool contract with the given interest-rate and
/// asset parameters.
pub(crate) fn pool_create_market_call(
    env: &Env,
    pool_addr: &Address,
    hub_id: u32,
    params: &MarketParamsRaw,
) {
    LiquidityPoolClient::new(env, pool_addr).create_market(&hub_id, params)
}

/// Submits a batch of supply requests to the pool contract, returning each entry's updated
/// scaled position and market indices.
pub(crate) fn pool_supply_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSupplyEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).supply(entries)
}

/// Submits a batch of borrow requests to the pool contract, which mints scaled debt and pays
/// out the borrowed assets to `receiver`.
pub(crate) fn pool_borrow_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    entries: &Vec<PoolBorrowEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).borrow(receiver, entries)
}

/// Opens a leveraged strategy position on the pool contract, borrowing `action`'s amount for
/// `receiver` and, when `charge_fee` is true, deducting a fee before the transfer.
pub(crate) fn pool_create_strategy_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    LiquidityPoolClient::new(env, pool_addr).create_strategy(receiver, &action, &charge_fee)
}

/// Submits a batch of withdrawal requests to the pool contract, sending the underlying assets
/// to `receiver`; `is_liquidation` skips utilization caps and lets each entry withhold a
/// protocol fee from the gross amount.
pub(crate) fn pool_withdraw_call(
    env: &Env,
    pool_addr: &Address,
    receiver: &Address,
    is_liquidation: bool,
    entries: &Vec<PoolWithdrawEntry>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).withdraw(receiver, &is_liquidation, entries)
}

/// Submits a batch of repay actions to the pool contract, burning scaled debt and refunding
/// any overpayment to `payer`.
pub(crate) fn pool_repay_call(
    env: &Env,
    pool_addr: &Address,
    payer: &Address,
    actions: &Vec<PoolAction>,
) -> Vec<PoolPositionMutation> {
    LiquidityPoolClient::new(env, pool_addr).repay(payer, actions)
}

/// Nets a user's supply against their debt on the same market via the pool contract, burning
/// the matched scaled amounts without moving cash.
pub(crate) fn pool_net_settle_call(
    env: &Env,
    pool_addr: &Address,
    entry: &PoolNetSettleEntry,
) -> PoolNetSettleResult {
    LiquidityPoolClient::new(env, pool_addr).net_settle(entry)
}

/// Seizes the given positions on the pool contract during liquidation or bad-debt cleanup:
/// borrow-side entries socialize the amount as bad debt, deposit-side entries reclassify it as
/// protocol revenue.
pub(crate) fn pool_seize_positions_call(
    env: &Env,
    pool_addr: &Address,
    entries: &Vec<PoolSeizeEntry>,
) {
    LiquidityPoolClient::new(env, pool_addr).seize_positions(entries)
}

/// Executes a flash loan of `amount` of `hub_asset` on the pool contract, sending funds to
/// `receiver` and returning the fee charged.
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

/// Accrues interest for each market in `hub_assets` on the pool contract through the current
/// ledger time.
pub(crate) fn pool_update_indexes_call(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) {
    LiquidityPoolClient::new(env, pool_addr).update_indexes(hub_assets)
}

/// Claims `hub_asset`'s accrued protocol revenue from the pool contract and returns the amount
/// transferred to the owner.
pub(crate) fn pool_claim_revenue_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).claim_revenue(hub_asset)
}

/// Injects up to `amount` of cash from `payer` into `hub_asset`'s market on the pool contract
/// to cover a backing shortfall, refunding any unused amount.
pub(crate) fn pool_recapitalize_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    payer: &Address,
    amount: i128,
) -> PoolAmountMutation {
    LiquidityPoolClient::new(env, pool_addr).recapitalize(hub_asset, payer, &amount)
}

/// Fetches `hub_asset`'s current market parameters and state from the pool contract.
pub(crate) fn fetch_pool_sync_data(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
) -> PoolSyncData {
    LiquidityPoolClient::new(env, pool_addr).get_sync_data(hub_asset)
}

/// Fetches simulated, up-to-date borrow and supply indexes for each of `hub_assets` from the
/// pool contract without writing state.
pub(crate) fn fetch_pool_bulk_indexes(
    env: &Env,
    pool_addr: &Address,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexRaw> {
    LiquidityPoolClient::new(env, pool_addr).get_bulk_indexes(hub_assets)
}

/// Replaces `hub_asset`'s interest-rate model and flash-loan settings on the pool contract.
pub(crate) fn pool_update_params_call(
    env: &Env,
    pool_addr: &Address,
    hub_asset: &HubAssetKey,
    params: &InterestRateModel,
) {
    LiquidityPoolClient::new(env, pool_addr).update_params(hub_asset, params)
}

/// Upgrades the pool contract's WASM to `new_wasm_hash`.
pub(crate) fn pool_upgrade_call(env: &Env, pool_addr: &Address, new_wasm_hash: &BytesN<32>) {
    LiquidityPoolClient::new(env, pool_addr).upgrade(new_wasm_hash)
}

#[cfg(test)]
#[path = "../../tests/external/pool.rs"]
mod tests;
