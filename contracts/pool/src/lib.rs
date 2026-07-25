#![no_std]

//! Owner-gated liquidity pool: interest, scaled shares, cash. The controller
//! owns solvency and risk; this contract owns its own books.
//!
//! Top level only declares modules and the ABI. Every entrypoint delegates to
//! the module in `ops` that owns that operation end to end.
//! See `docs/reference/invariants.md`.

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
    /// Sets the contract owner once. Called by the deploying factory.
    ///
    /// # Arguments
    /// * `admin` — address granted owner rights; must be the lending controller.
    ///
    /// # Security Warning
    /// * Runs without authorization and can set the owner only once; the
    ///   deploying factory must pass the trusted controller address.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    /// Creates a market with `params` and zeroed state (indexes = `RAY`). Owner
    /// (controller) only.
    ///
    /// # Errors
    /// * `AssetAlreadySupported` — params already exist for `(hub_id, asset)`.
    /// * `AssetDecimalsTooHigh` — `asset_decimals` exceeds `RAY_DECIMALS`.
    /// * `InvalidBorrowParams` — `flashloan_fee` exceeds the protocol cap.
    /// * `BaseRateNegative` / `SlopeNonMonotonic` / `MaxRateBelowBase` /
    ///   `MaxBorrowRateTooHigh` / `InvalidUtilRange` / `OptUtilTooHigh` /
    ///   `InvalidReserveFactor` — rate-model bounds from `InterestRateModel::verify`.
    /// * `MathOverflow` — ledger timestamp to ms overflow.
    ///
    /// # Events
    /// * topics — `["market", "batch_params_update"]`
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    /// Supplies each entry and mints scaled shares, returning input-ordered
    /// position mutations. Owner (controller) only. The controller must
    /// pre-transfer the tokens.
    ///
    /// # Arguments
    /// * `entries` — one supply leg per entry; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount is negative.
    /// * `PoolInsolvent` — aggregate supply claims exceed cash plus debt.
    /// * `SupplyRoundsToZeroShares` — a positive supply mints zero shares at
    ///   the current index.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no account health check; the controller must gate the supply.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, ops::supply::apply)
    }

    /// Borrows each entry to `receiver`, returning input-ordered position
    /// mutations. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — proceeds recipient for every leg.
    /// * `entries` — one borrow leg per entry; amounts must be positive.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount is not strictly positive.
    /// * `BorrowRoundsToZeroShares` — a positive amount mints zero scaled debt
    ///   despite ceil rounding.
    /// * `InsufficientLiquidity` — tracked cash cannot cover the borrow.
    /// * `UtilizationAboveMax` — the borrow pushes utilization past the market cap.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency or collateral check; the owning
    ///   controller must gate the borrow against account health.
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

    /// Withdraws each entry to `receiver`, returning input-ordered position
    /// mutations. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — recipient of the net withdrawal for every leg.
    /// * `is_liquidation` — applies to the whole call; enables the protocol fee
    ///   and skips the max-utilization check for liquidation seizures.
    /// * `entries` — one withdraw leg per entry; a full-position sentinel amount
    ///   closes the position; `protocol_fee` must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `AmountMustBePositive` — an entry amount or `protocol_fee` is negative.
    /// * `WithdrawLessThanFee` — the liquidation fee exceeds the gross seized amount.
    /// * `WithdrawRoundsToZeroShares` — a positive withdrawal burns zero scaled
    ///   supply despite ceil rounding.
    /// * `InsufficientLiquidity` — tracked cash cannot cover the net transfer.
    /// * `UtilizationAboveMax` — a non-liquidation withdrawal breaches the utilization cap.
    /// * `PoolInsolvent` — the projected state leaves debt with zero supply.
    /// * `MathOverflow` — scaled-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency check; the owning controller must confirm
    ///   the account stays healthy after the withdrawal.
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

    /// Repays each action and refunds overpayments to `payer`, returning
    /// input-ordered position mutations. Owner (controller) only. The
    /// controller must pre-transfer the repayment tokens.
    ///
    /// # Arguments
    /// * `payer` — recipient of any overpayment refund.
    /// * `actions` — one repay leg per action; amounts must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an action targets a market with no stored state.
    /// * `AmountMustBePositive` — an action amount is negative.
    /// * `RepayRoundsToZeroShares` — a positive applied repayment burns zero
    ///   scaled debt at the current index.
    /// * `MathOverflow` — debt-share or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    /// Accrues interest for `hub_asset` and persists indexes. Owner (controller)
    /// only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `MathOverflow` — accrual or timestamp math overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        ops::market::accrue(&env, hub_asset);
    }

    /// Distributes `amount` to suppliers by growing the supply index. Owner
    /// (controller) only. The controller must pre-transfer the reward tokens.
    ///
    /// # Arguments
    /// * `amount` — reward tokens to distribute; must be non-negative.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `AmountMustBePositive` — `amount` is negative.
    /// * `NoSuppliersToReward` — the market has no scaled supply to receive rewards.
    /// * `SupplyIndexRewardCeiling` — the reward would push the supply index
    ///   past its cap.
    /// * `MathOverflow` — index or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn add_rewards(env: Env, hub_asset: HubAssetKey, amount: i128) {
        ops::rewards::apply(&env, hub_asset, amount);
    }

    /// Lends `amount` to `receiver`, invokes its `execute_flash_loan` callback,
    /// and pulls back `amount + fee`; the fee (from market `flashloan_fee` bps)
    /// becomes protocol revenue. Owner (controller) only. Returns the fee.
    ///
    /// # Arguments
    /// * `initiator` — forwarded to the receiver callback as the loan originator.
    /// * `receiver` — deployed Wasm contract that receives the loan and repays it.
    /// * `amount` — loaned amount; must be positive.
    /// * `data` — opaque callback payload forwarded to the receiver.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `FlashloanNotEnabled` — the market is not flashloanable.
    /// * `InsufficientLiquidity` — tracked cash cannot fund the loan.
    /// * `InvalidFlashloanReceiver` — `receiver` is not a deployed Wasm contract.
    /// * `InvalidFlashloanRepay` — payout, callback, allowance, or repayment leaves
    ///   the pool's loaned-token balance off its expected value.
    /// * `MathOverflow` — loan, fee, or balance accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Bridges an external callback: repayment is enforced solely by loaned-token
    ///   balance and allowance checks that bracket the callback and `transfer_from`,
    ///   so the asset must be a well-behaved SAC.
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

    /// Opens a strategy borrow: mints scaled debt, books the market flash-loan
    /// fee as protocol revenue when `charge_fee`, and transfers `amount - fee`
    /// to `receiver`. Owner (controller) only.
    ///
    /// # Arguments
    /// * `receiver` — recipient of the net (post-fee) borrowed amount.
    /// * `action` — the strategy borrow leg; amount must be positive.
    /// * `charge_fee` — when true, withhold the market `flashloan_fee` bps;
    ///   when false (migration), borrow fee-free.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for the action's market.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `StrategyFeeExceeds` — computed fee exceeds the borrowed `amount`.
    /// * `BorrowRoundsToZeroShares` — a positive amount mints zero scaled debt.
    /// * `InsufficientLiquidity` — tracked cash cannot fund the borrow.
    /// * `UtilizationAboveMax` — the borrow pushes utilization past the market cap.
    /// * `MathOverflow` — scaled-debt, fee, or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["strategy", "fee"]` (suppressed when fee is zero)
    /// * topics — `["market", "batch_state_update"]`
    ///
    /// # Security Warning
    /// * Performs no borrower solvency check and enforces no spoke borrow cap; the
    ///   owning controller must gate the strategy against account health and caps.
    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        ops::strategy::apply(&env, &receiver, action, charge_fee)
    }

    /// Seizes positions: borrow legs write down the supply index for bad debt;
    /// deposit legs move dust into revenue. Owner (controller) only. Duplicate
    /// hub-assets in one batch apply sequentially.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — an entry targets a market with no stored state.
    /// * `InternalError` — a seized deposit exceeds the market's supply shares.
    /// * `MathOverflow` — bad-debt, revenue, or scaled-total accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch_without_result(&env, entries, ops::seize::apply);
    }

    /// Nets a supply leg against a debt leg on the same hub-asset with zero
    /// token transfer. Settles the lesser of `entry.amount`, supply balance,
    /// and debt owed; leftover collateral stays as supply. Owner (controller)
    /// only.
    ///
    /// # Arguments
    /// * `entry` — hub-asset market plus both legs' current scaled amounts.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — the entry targets a market with no stored state.
    /// * `AmountMustBePositive` — `entry.amount` is negative.
    /// * `InternalError` — the repay leg overpaid (structurally unexpected).
    /// * `NetSettleRoundsToZeroShares` — a positive settlement burns zero scaled
    ///   units on either leg.
    /// * `PoolInsolvent` — the projected state leaves debt with zero supply.
    /// * `MathOverflow` — scaled-share accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        storage::renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Burns claimable protocol revenue shares and transfers the floored cash
    /// payout to the owner. Owner (controller) only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `UtilizationAboveMax` — the claim would leave utilization above the cap.
    /// * `PoolInsolvent` — the projected state leaves debt with zero supply.
    /// * `OwnerNotSet` — claimable amount is positive but no owner is configured.
    /// * `MathOverflow` — revenue or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        ops::revenue::apply(&env, hub_asset)
    }

    /// Reconciles tracked `cash` to the live SAC balance after an issuer
    /// clawback, socializing the shortfall through the supply index. Owner
    /// (controller) only. No-op unless `cash > balance`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `MathOverflow` — accrual or cash accounting overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_state_update"]`
    #[only_owner]
    fn reconcile_reserves(env: Env, hub_asset: HubAssetKey) {
        ops::reconcile::apply(&env, hub_asset);
    }

    /// Accrues at the current rate model, then replaces the interest-rate
    /// parameters for `hub_asset`. Owner (controller) only.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — no stored state for `hub_asset`.
    /// * `BaseRateNegative` / `SlopeNonMonotonic` / `MaxRateBelowBase` /
    ///   `MaxBorrowRateTooHigh` / `InvalidUtilRange` / `OptUtilTooHigh` /
    ///   `InvalidReserveFactor` — rate-model bounds from `InterestRateModel::verify`.
    /// * `MathOverflow` — accrual or timestamp math overflows.
    ///
    /// # Events
    /// * topics — `["market", "batch_params_update"]`
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    /// Replaces the pool contract Wasm with the code at `new_wasm_hash`. Owner
    /// (controller) only.
    ///
    /// # Arguments
    /// * `new_wasm_hash` — hash of already-installed Wasm to run on next invocation.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    // --- views: unauthenticated checkpoint reads ---

    /// Returns checkpoint utilization in RAY for `hub_asset` (no accrual). The
    /// ABI keeps the British spelling; everything else spells it `utilization`.
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::utilization(&env, &hub_asset)
    }

    /// Returns tracked `cash` in asset decimals (not live SAC balance).
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    /// Returns the checkpoint deposit rate in RAY (no accrual).
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Returns the checkpoint borrow rate in RAY (no accrual).
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Returns floored claimable protocol revenue in asset decimals.
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    /// Returns total supplied amount in asset decimals (checkpoint, no accrual).
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    /// Returns total borrowed amount in asset decimals (checkpoint, no accrual).
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    /// Returns milliseconds since the market last accrued interest.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    /// Returns raw params and accounting state (checkpoint). Prefer
    /// `get_bulk_indexes` for live indexes.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        storage::load_sync_data(&env, &hub_asset)
    }

    /// Returns borrow/supply indexes accrued to now for each hub-asset (simulate,
    /// no write).
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
