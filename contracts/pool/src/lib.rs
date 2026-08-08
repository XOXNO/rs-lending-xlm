#![no_std]
//! # Liquidity Pool contract
//!
//! Soroban contract that holds per-asset market state (cash, scaled supply/debt,
//! interest indexes, protocol revenue) and executes market mutations.
//!
//! ## Architecture
//!
//! The hub (owner) is the only party allowed to mutate state. End users never
//! call these entrypoints directly in production flows; the hub orchestrates
//! transfers, position books, and risk checks, then invokes the pool.
//!
//! | Layer | Role |
//! |-------|------|
//! | [`LiquidityPool`] / [`LiquidityPoolInterface`] | Public entrypoints, owner gates |
//! | [`ops`] | Mutation legs (supply, borrow, repay, …) |
//! | [`cache::Cache`] | In-memory market view + commit |
//! | [`interest`] | Index accrual and fee socialization |
//! | [`guards`] | Utilization and solvency checks |
//! | [`storage`] | Persistent params/state + TTL bumps |
//! | [`views`] | Read-only rate and balance queries |
//!
//! ## Accounting model
//!
//! Positions are stored as **scaled shares** (RAY fixed-point). Asset amounts
//! convert via the market's supply or borrow index. Indexes grow over time as
//! interest accrues; protocol revenue is held as scaled supply shares so it
//! earns the same supplier rate until claimed.
//!
//! ## Security notes
//!
//! - All mutators (except views) require the contract owner via `#[only_owner]`.
//! - Cash is tracked separately from token balances; flash loans verify the
//!   on-chain balance after payout and after repayment.
//! - Instance and market storage TTLs are extended on write paths.

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

/// Deployed liquidity pool instance.
///
/// Holds no methods of its own beyond construction; market operations live on
/// [`LiquidityPoolInterface`].
#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    /// Initialize the pool and set `admin` as the Ownable owner.
    ///
    /// Called once at deploy. Subsequent mutations require that owner (normally
    /// the hub contract).
    ///
    /// # Arguments
    ///
    /// * `admin` - Address that may call `#[only_owner]` entrypoints.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    /// Create a new asset market under `hub_id` with the given rate parameters.
    ///
    /// Initializes indexes at RAY (1.0), zero cash/supply/debt, and the current
    /// ledger timestamp. Panics if the hub-asset pair already exists.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    /// Replace the interest-rate curve and flash-loan settings for a market.
    ///
    /// Accrues interest first so the old curve is applied through the current
    /// ledger, then writes the new model into market params.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    /// Upgrade the contract WASM to `new_wasm_hash`.
    ///
    /// Extends instance TTL before invoking the upgradeable helper.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    /// Batch-supply assets into one or more markets.
    ///
    /// For each entry: accrue, mint scaled supply shares, credit cash. The hub
    /// is expected to have already transferred the tokens into the pool.
    ///
    /// # Returns
    ///
    /// One [`PoolPositionMutation`] per entry (updated scaled position + indexes).
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, ops::supply::apply)
    }

    /// Batch-borrow assets and transfer them to `receiver`.
    ///
    /// For each entry: accrue, mint scaled debt, debit cash, transfer tokens out.
    /// Enforces max utilization after each mint.
    ///
    /// # Returns
    ///
    /// One [`PoolPositionMutation`] per entry.
    ///
    /// # Authorization
    ///
    /// Owner only.
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

    /// Batch-withdraw supply shares and transfer underlying to `receiver`.
    ///
    /// When `is_liquidation` is true, utilization caps are skipped and an optional
    /// protocol fee may be withheld from the gross withdrawal.
    ///
    /// # Returns
    ///
    /// One [`PoolPositionMutation`] per entry (gross amount in `actual_amount`).
    ///
    /// # Authorization
    ///
    /// Owner only.
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

    /// Batch-repay debt; refund overpayment to `payer`.
    ///
    /// For each action: accrue, burn scaled debt up to the repay amount, credit
    /// cash with the net repay, transfer any overpayment back to `payer`.
    ///
    /// # Returns
    ///
    /// One [`PoolPositionMutation`] per action (`actual_amount` is net repay).
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    /// Accrue interest for a single market through the current ledger time.
    ///
    /// No-op write if no time has elapsed; still emits a market state event.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn update_indexes(env: Env, hub_asset: HubAssetKey) {
        ops::market::accrue(&env, hub_asset);
    }

    /// Inject cash to cover a backing shortfall; refund unused amount to `payer`.
    ///
    /// Only applies up to [`guards::backing_shortfall`]. Excess is returned via
    /// token transfer.
    ///
    /// # Returns
    ///
    /// [`PoolAmountMutation`] with the amount actually applied.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation {
        ops::recapitalize::apply(&env, hub_asset, payer, amount)
    }

    /// Execute a flash loan of `amount` of `hub_asset` to `receiver`.
    ///
    /// Transfers funds out, invokes `execute_flash_loan` on the receiver WASM,
    /// pulls principal + fee via `transfer_from`, and books the fee as protocol
    /// revenue. Market must have flash loans enabled.
    ///
    /// # Returns
    ///
    /// Fee charged in asset units.
    ///
    /// # Authorization
    ///
    /// Owner only.
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

    /// Open a strategy (borrow-style) position, optionally charging a flash fee.
    ///
    /// Mints debt for `action.amount`, books fee as protocol revenue when
    /// `charge_fee` is true, and transfers `amount - fee` to `receiver`.
    ///
    /// # Returns
    ///
    /// [`PoolStrategyMutation`] with scaled position, gross amount, and net sent.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        ops::strategy::apply(&env, &receiver, action, charge_fee)
    }

    /// Seize positions during liquidation or bad-debt cleanup.
    ///
    /// Borrow-side: socializes bad debt onto the supply index and burns debt.
    /// Deposit-side: reclassifies supply shares as protocol revenue.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch_without_result(&env, entries, ops::seize::apply);
    }

    /// Net a user's supply against their debt on the same market (no cash move).
    ///
    /// Burns matched scaled supply and debt up to `entry.amount` (capped by debt
    /// value). Leaves residual positions in the result.
    ///
    /// # Authorization
    ///
    /// Owner only.
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        storage::renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Claim accrued protocol revenue and transfer it to the contract owner.
    ///
    /// Burns claimable revenue shares, debits cash, and sends tokens to the
    /// Ownable owner. Returns zero amount if nothing is claimable.
    ///
    /// # Authorization
    ///
    /// Owner only (caller). Funds are paid to the Ownable owner address.
    #[only_owner]
    fn claim_revenue(env: Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
        ops::revenue::apply(&env, hub_asset)
    }

    /// Current utilization ratio for a market (RAY fixed-point raw value).
    ///
    /// Computed from stored indexes without forcing a state write.
    fn get_utilisation(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::utilization(&env, &hub_asset)
    }

    /// Available cash reserves (asset units) for a market.
    fn get_reserves(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::reserves(&env, &hub_asset)
    }

    /// Current supplier APY-style rate (RAY raw) at the stored utilization.
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Current borrow rate (RAY raw) at the stored utilization.
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Claimable protocol revenue in asset units (floored unscale of revenue shares).
    fn get_revenue(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::protocol_revenue(&env, &hub_asset)
    }

    /// Total supplied underlying in asset units (from scaled supply × supply index).
    fn get_supplied_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::supplied_amount(&env, &hub_asset)
    }

    /// Total borrowed underlying in asset units (from scaled debt × borrow index).
    fn get_borrowed_amount(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrowed_amount(&env, &hub_asset)
    }

    /// Milliseconds since the market's last interest accrual timestamp.
    fn get_delta_time(env: Env, hub_asset: HubAssetKey) -> u64 {
        views::delta_time(&env, &hub_asset)
    }

    /// Full market params + state blob used for hub sync / off-chain indexing.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        storage::load_sync_data(&env, &hub_asset)
    }

    /// Simulate accrued indexes for many markets without writing state.
    ///
    /// For each key, loads sync data and runs [`simulate_update_indexes`] to the
    /// current ledger time. Useful for the hub to refresh position valuations.
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
