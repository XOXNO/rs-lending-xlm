#![no_std]

//! Owner-gated liquidity pool: interest, scaled shares, cash. The controller
//! owns solvency and risk; this contract owns its own books.
//!
//! Top level declares modules and the ABI. State-changing operations delegate
//! to the `ops` module that owns that operation end to end. Invariants live
//! next to the checks that enforce them (`guards`, `cache` cash comments).

mod cache;
mod events;
mod guards;
mod interest;
mod ops;
mod storage;
mod time;
mod views;

#[cfg(test)]
#[path = "../tests/test_support.rs"]
mod test_support;

#[cfg(feature = "certora")]
#[path = "../../../certora/pool/spec/mod.rs"]
pub mod spec;

use common::rates::simulate_update_indexes;
use common::types::{
    HubAssetKey, InterestRateModel, MarketIndexRaw, MarketParamsRaw, PoolAction,
    PoolAmountMutation, PoolBorrowEntry, PoolNetSettleEntry, PoolNetSettleResult,
    PoolPositionMutation, PoolSeizeEntry, PoolStrategyMutation, PoolSupplyEntry, PoolSyncData,
    PoolWithdrawEntry,
};

use pool_interface::LiquidityPoolInterface;

use soroban_sdk::{contract, contractimpl, contractmeta, Address, Bytes, BytesN, Env, Vec};

use stellar_access::ownable;
use stellar_macros::only_owner;

contractmeta!(key = "name", val = "Liquidity Pool");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct LiquidityPool;

// Soroban constructors cannot be declared in contractclient traits.
#[contractimpl]
impl LiquidityPool {
    /// Sets the owner once (deploying factory must pass the controller).
    ///
    /// # Security Warning
    /// * No auth; runs only once. Factory must pass the trusted controller.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    // --- market lifecycle ---

    /// Creates a market with `params` and zeroed state (indexes = `RAY`).
    ///
    /// # Errors
    /// * `AssetAlreadySupported` · `AssetDecimalsTooHigh` · `InvalidBorrowParams`
    /// * rate-model bounds from `InterestRateModel::verify`
    /// * `MathOverflow`
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    /// Accrues, then replaces the rate model / flash-loan flags for `hub_asset`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · rate-model bounds · `InvalidBorrowParams` · `MathOverflow`
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    /// Replaces contract Wasm at `new_wasm_hash` (already installed).
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    // --- money paths (controller pre-transfers where noted) ---

    /// Mints supply shares; controller must pre-transfer tokens.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `PoolInsolvent`
    /// * `SupplyRoundsToZeroShares` · `MathOverflow`
    ///
    /// # Security Warning
    /// * No account health check; controller must gate the supply.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, ops::supply::apply)
    }

    /// Borrows each entry to `receiver`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `BorrowRoundsToZeroShares`
    /// * `InsufficientLiquidity` · `UtilizationAboveMax` · `MathOverflow`
    ///
    /// # Security Warning
    /// * No solvency/collateral check; controller must gate against account health.
    #[only_owner]
    fn borrow(
        env: Env,
        receiver: Address,
        entries: Vec<PoolBorrowEntry>,
    ) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, |env, entry| {
            ops::borrow::apply(env, &receiver, entry)
        })
    }

    /// Withdraws each entry to `receiver`. Full-close when amount ≥ half-up balance.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `WithdrawLessThanFee`
    /// * `WithdrawRoundsToZeroShares` · `InternalError` · `InsufficientLiquidity`
    /// * `UtilizationAboveMax` · `PoolInsolvent` · `MathOverflow`
    ///
    /// # Security Warning
    /// * No solvency check; controller must confirm health after withdrawal.
    #[only_owner]
    fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, |env, entry| {
            ops::withdraw::apply(env, &receiver, is_liquidation, entry)
        })
    }

    /// Repays each action; refunds overpayments to `payer`. Controller pre-transfers.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `RepayRoundsToZeroShares`
    /// * `MathOverflow`
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    /// Accrues interest for `hub_asset` when ledger time has elapsed.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `MathOverflow`
    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        ops::market::accrue(&env, hub_asset);
    }

    /// Grows the supply index by a pre-transferred reward `amount`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `NoSuppliersToReward`
    /// * `SupplyIndexRewardCeiling` · `MathOverflow`
    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        ops::rewards::apply(&env, hub_asset, amount);
    }

    /// Covers backing shortfall with pre-transferred cash; refunds excess to `payer`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `MathOverflow`
    #[only_owner]
    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation {
        ops::recapitalize::apply(&env, hub_asset, payer, amount)
    }

    /// Flash-loans `amount` to a Wasm `receiver`; fee becomes protocol revenue.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `FlashloanNotEnabled`
    /// * `InsufficientLiquidity` · `InvalidFlashloanReceiver` · `InvalidFlashloanRepay`
    /// * `MathOverflow`
    ///
    /// # Security Warning
    /// * External callback: exact SAC balance checks after payout, callback, and
    ///   `transfer_from`. Asset must be a well-behaved SAC.
    #[only_owner]
    fn flash_loan(
        env: Env,
        hub_asset: HubAssetKey,
        initiator: Address,
        receiver: Address,
        amount: i128,
        data: Bytes,
    ) -> i128 {
        ops::flash::apply(&env, hub_asset, initiator, receiver, amount, data)
    }

    /// Strategy borrow: mints debt, optional flash fee as revenue, pays net out.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `StrategyFeeExceeds`
    /// * `BorrowRoundsToZeroShares` · `InsufficientLiquidity` · `UtilizationAboveMax`
    /// * `MathOverflow`
    ///
    /// # Security Warning
    /// * No solvency or spoke borrow-cap check; controller must gate both.
    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        ops::strategy::apply(&env, &receiver, action, charge_fee)
    }

    /// Seize: borrow legs write down supply index; deposits → protocol revenue.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `InternalError` · `MathOverflow`
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch_without_result(&env, entries, ops::seize::apply);
    }

    /// Nets supply against debt on one hub-asset with zero token transfer.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `AmountMustBePositive` · `InternalError`
    /// * `NetSettleRoundsToZeroShares` · `PoolInsolvent` · `MathOverflow`
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        storage::renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Burns protocol revenue shares; floored payout capped by tracked cash.
    ///
    /// # Errors
    /// * `PoolNotInitialized` · `UtilizationAboveMax` · `PoolInsolvent` · `MathOverflow`
    /// * `InternalError` — a cash-short claim rounds to zero shares burned
    ///   despite a positive payout, at extreme cash-to-claim ratios.
    #[only_owner]
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        ops::revenue::apply(&env, hub_asset)
    }

    // --- views: unauthenticated checkpoint reads (no accrual unless noted) ---

    /// Checkpoint utilization in RAY. ABI keeps British spelling.
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::utilization(&env, &hub_asset)
    }

    /// Tracked `cash` in asset decimals (not live SAC balance).
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    /// Checkpoint deposit rate in RAY.
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Checkpoint borrow rate in RAY.
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Floored underlying of protocol revenue shares (not cash-capped).
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    /// Total supplied amount in asset decimals (checkpoint).
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    /// Total borrowed amount in asset decimals (checkpoint).
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    /// Milliseconds since the market last accrued.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    /// Raw params and state (checkpoint). Prefer `get_bulk_indexes` for live indexes.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        storage::load_sync_data(&env, &hub_asset)
    }

    /// Borrow/supply indexes accrued to now (simulate, no write).
    fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let now = time::now_ms(&env);
        let mut indexes = Vec::new(&env);
        for hub_asset in hub_assets.iter() {
            let sync = storage::load_sync_data(&env, &hub_asset);
            indexes.push_back(MarketIndexRaw::from(&simulate_update_indexes(
                &env, now, &sync,
            )));
        }
        indexes
    }
}

#[cfg(test)]
#[path = "../tests/lib_orchestration.rs"]
mod lib_orchestration_tests;

#[cfg(test)]
#[path = "../tests/flows.rs"]
mod tests;
