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
//! | `ops` | Mutation legs (supply, borrow, repay, …) |
//! | `cache::Cache` | In-memory market view + commit |
//! | `interest` | Index accrual and fee socialization |
//! | `guards` | Utilization and solvency checks |
//! | `storage` | Persistent params/state + TTL bumps |
//! | `views` | Read-only rate and balance queries |
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
use common::ttl::renew_instance;
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
    /// Sets `admin` as the Ownable owner at construction. Every
    /// `#[only_owner]` entrypoint afterward requires that owner, normally the
    /// hub contract, to authorize.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    /// Creates a new asset market under `hub_id` with the given rate
    /// parameters. Initializes indexes at RAY (1.0) with zero cash, supply,
    /// and debt; panics if the hub-asset pair already exists. Restricted to
    /// the owner.
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    /// Replaces the interest-rate curve and flash-loan settings for a
    /// market. Accrues interest first so the old curve applies through the
    /// current ledger, then writes the new model into market params.
    /// Restricted to the owner.
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    /// Upgrades the contract WASM to `new_wasm_hash`, extending instance TTL
    /// first. Restricted to the owner.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_instance(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    /// Batch-supplies assets into one or more markets: accrues interest,
    /// mints scaled supply shares, and credits cash per entry. The hub is
    /// expected to have already transferred the tokens into the pool.
    /// Restricted to the owner; returns one [`PoolPositionMutation`] per
    /// entry.
    #[only_owner]
    fn supply(env: Env, entries: Vec<PoolSupplyEntry>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, entries, ops::supply::apply)
    }

    /// Batch-borrows assets and transfers them to `receiver`: accrues
    /// interest, mints scaled debt, debits cash, and enforces max
    /// utilization after each mint. Restricted to the owner; returns one
    /// [`PoolPositionMutation`] per entry.
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

    /// Batch-withdraws supply shares and transfers the underlying to
    /// `receiver`. When `is_liquidation` is true, utilization caps are
    /// skipped and an optional protocol fee may be withheld from the gross
    /// withdrawal. Restricted to the owner; returns one
    /// [`PoolPositionMutation`] per entry with the gross amount in
    /// `actual_amount`.
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

    /// Batch-repays debt and refunds any overpayment to `payer`: accrues
    /// interest, burns scaled debt up to the repay amount, and credits cash
    /// with the net repay per action. Restricted to the owner; returns one
    /// [`PoolPositionMutation`] per action with `actual_amount` set to the
    /// net repay.
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    /// Accrues interest for each market in `hub_assets` through the current
    /// ledger time. No-op write for a market with no elapsed time, but still
    /// emits its market state event. Restricted to the owner.
    #[only_owner]
    fn update_indexes(env: Env, hub_assets: Vec<HubAssetKey>) {
        ops::market::accrue(&env, hub_assets);
    }

    /// Injects cash to cover a market's backing shortfall and refunds any
    /// unused amount to `payer`. Applies at most `guards::backing_shortfall`,
    /// returning the excess via token transfer. Restricted to the owner;
    /// returns a [`PoolAmountMutation`] with the amount actually applied.
    #[only_owner]
    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation {
        ops::recapitalize::apply(&env, hub_asset, payer, amount)
    }

    /// Executes a flash loan of `amount` of `hub_asset` to `receiver`,
    /// requiring the market to have flash loans enabled. Transfers the funds
    /// out, invokes `execute_flash_loan` on the receiver WASM, pulls
    /// principal plus fee back via `transfer_from`, and books the fee as
    /// protocol revenue. Restricted to the owner; returns the fee charged in
    /// asset units.
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

    /// Opens a strategy (borrow-style) position, optionally charging a
    /// flash fee. Mints debt for `action.amount`, books the fee as protocol
    /// revenue when `charge_fee` is true, and transfers `amount - fee` to
    /// `receiver`. Restricted to the owner; returns a
    /// [`PoolStrategyMutation`] with the scaled position, gross amount, and
    /// net amount sent.
    #[only_owner]
    fn create_strategy(
        env: Env,
        receiver: Address,
        action: PoolAction,
        charge_fee: bool,
    ) -> PoolStrategyMutation {
        ops::strategy::apply(&env, &receiver, action, charge_fee)
    }

    /// Seizes positions during liquidation or bad-debt cleanup. Borrow-side
    /// entries socialize bad debt onto the supply index and burn the debt;
    /// deposit-side entries reclassify supply shares as protocol revenue.
    /// Restricted to the owner.
    #[only_owner]
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch(&env, entries, |e, entry| ((), ops::seize::apply(e, entry)));
    }

    /// Nets a user's supply against their debt on the same market with no
    /// cash movement. Burns matched scaled supply and debt up to
    /// `entry.amount`, capped by the conservative overlap of floored supply
    /// and ceiled debt, and returns the residual positions. Restricted to
    /// the owner.
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Claims accrued protocol revenue and transfers it to the contract
    /// owner. Burns claimable revenue shares, debits cash, and sends the
    /// tokens to the Ownable owner; returns a zero amount if nothing is
    /// claimable. Restricted to the owner, who is also the recipient of the
    /// funds.
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

    /// Current supplier APR as annual RAY at the stored utilization.
    ///
    /// Divide by `RAY` for a unit fraction (0.05 = 5%). Accrual still compounds
    /// the per-millisecond form of this rate.
    fn get_deposit_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::deposit_rate(&env, &hub_asset)
    }

    /// Current borrow APR as annual RAY at the stored utilization.
    ///
    /// Divide by `RAY` for a unit fraction (0.05 = 5%). Accrual still compounds
    /// the per-millisecond form of this rate.
    fn get_borrow_rate(env: Env, hub_asset: HubAssetKey) -> i128 {
        views::borrow_rate(&env, &hub_asset)
    }

    /// Protocol revenue in asset units at the **stored** supply index (floored
    /// unscale of revenue shares). This view does not accrue interest first.
    ///
    /// `claim_revenue` syncs the market before paying, so it pays
    /// `min(cash, revenue)` computed on post-accrual state. The amount actually
    /// paid can therefore be higher than this value (accrual pending since the
    /// last market write mints further revenue shares) or lower (the cash cap
    /// binds when the market is heavily utilized).
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
