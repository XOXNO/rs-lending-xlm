#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only ABI mirror of the liquidity-pool contract.
//!
//! `#[contractclient]` generates `LiquidityPoolClient`. Matches deployed pool
//! entrypoints by ABI name (no formal `impl`).

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env, Vec};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw);

    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>;

    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Withdraws each entry to `receiver`. Full position at `i128::MAX` sentinel.
    /// `is_liquidation` applies to the whole call; input-ordered.
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Tokens pre-transferred by controller; refunds overpayments to `payer`.
    /// Input-ordered.
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>;
    fn update_indexes(env: Env, hub_asset: HubAssetKey);
    /// External supply rewards for a hub-asset market.
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128);
    /// Must return `amount + fee`.
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    );

    /// `charge_fee`: apply the market's configured flash-loan fee, or borrow
    /// fee-free (migration). The fee amount is computed pool-side from the
    /// market's `flashloan_fee` bps.
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation;

    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>);

    /// Nets one supply leg against one debt leg on the same hub-asset with zero
    /// cash movement. Settled amount is min(request, supply, debt); leftover
    /// collateral beyond outstanding debt stays as supply. `supplied - borrowed`
    /// (== cash) is invariant.
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult;

    /// Protocol revenue capped by reserves and claimable shares.
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation;
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Accounted `cash` (asset decimals), not live token balance — donations
    /// cannot inflate it.
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128;
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128;
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128;
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128;
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128;
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Seconds since the market last accrued interest.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64;
    /// Raw params and accounting for one market. Index reads use `get_bulk_indexes`.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData;
    /// Borrow/supply indexes accrued to current ledger time, aligned with request.
    /// One call replaces N per-asset reads when only indexes are needed.
    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>;
}
