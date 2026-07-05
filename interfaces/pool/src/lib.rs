#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only ABI mirror of the liquidity-pool contract.
//!
//! `#[contractclient]` generates `LiquidityPoolClient` for typed cross-contract
//! calls. Mirrors the deployed pool's entrypoints 1:1; the pool matches these by
//! ABI name rather than formally implementing this trait.

use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};
use soroban_sdk::{contractclient, Address, Bytes, BytesN, Env, Vec};

#[contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPoolInterface {
    /// Creates an asset market on `hub_id` with fresh RAY indexes.
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw);

    /// Supplies each entry and returns the updated scaled shares, input-
    /// ordered. No counterparty: the controller pre-transfers the tokens.
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation>;

    /// Borrows each entry, transferring tokens to `receiver`; input-ordered.
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Withdraws each entry (full position at the i128::MAX sentinel) to
    /// `receiver`; `is_liquidation` applies to the whole call; input-ordered.
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation>;

    /// Repays each action (tokens pre-transferred by the controller),
    /// refunding overpayments to `payer`; input-ordered.
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation>;
    /// Accrues interest to the current ledger for one hub-asset market.
    fn update_indexes(env: Env, hub_asset: HubAssetKey);
    /// Adds `amount` of external supply rewards to a hub-asset market.
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128);
    /// Executes a flash loan that must return `amount + fee`.
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        fee: i128,
        data: Bytes,
    );

    /// Creates strategy debt and transfers `action.amount - fee` to `receiver`.
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        fee: i128,
    ) -> PoolStrategyMutation;

    /// Removes seized liquidation or bad-debt positions; entries targeting the
    /// same hub-asset are applied sequentially.
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>);

    /// Nets one supply leg against one debt leg on the same hub-asset with
    /// zero cash movement — the withdrawal and repayment settle for the
    /// identical real amount, so `supplied - borrowed` (== cash) is
    /// invariant. Caps the settled amount to the lesser of the request, the
    /// supply balance, and the debt owed; any leftover collateral beyond
    /// outstanding debt is left untouched as supply.
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult;

    /// Claims protocol revenue capped by reserves and claimable shares.
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation;
    /// Replaces the interest-rate model for a hub-asset market.
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel);
    /// Upgrades the pool contract to `new_wasm_hash`.
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    /// Returns the current utilisation for a hub-asset market.
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Available reserves = accounted `cash` (asset decimals), not the live token
    /// balance, so direct donations cannot inflate it.
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns the current deposit (supply) rate for a hub-asset market.
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns the current borrow rate for a hub-asset market.
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns accrued protocol revenue for a hub-asset market.
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns the total supplied underlying for a hub-asset market.
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns the total borrowed underlying for a hub-asset market.
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128;
    /// Returns seconds since the market last accrued interest.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64;
    /// Raw params and accounting state for one hub-asset market. Used for pool
    /// params (decimals, utilization caps); index reads go through `get_bulk_indexes`.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData;
    /// Borrow/supply indexes accrued to the current ledger time for each hub-asset
    /// market, index-aligned with the request. One call replaces N per-asset reads
    /// for flows that only need indexes.
    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw>;
}
