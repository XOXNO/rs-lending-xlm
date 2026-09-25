#![no_std]
//! # Liquidity Pool contract
//!
//! Soroban contract that holds per-market state (cash, scaled supply and debt,
//! interest indexes, protocol revenue) and executes market mutations.
//!
//! ## Architecture
//!
//! The owner, the controller, is the only caller that can mutate state
//! (INV-AUTH-01). The controller moves tokens, keeps the position books and
//! runs risk checks, then calls the pool.
//!
//! | Layer | Role |
//! |-------|------|
//! | [`LiquidityPool`] / [`LiquidityPoolInterface`] | Public entrypoints, owner gates |
//! | `ops` | Mutation legs (supply, borrow, repay, …) |
//! | `cache::Cache` | In-memory market view + commit |
//! | `interest` | Index accrual, revenue booking and bad-debt socialization |
//! | `guards` | Utilization and solvency checks |
//! | `storage` | Persistent params/state + TTL bumps |
//! | `views` | Read-only rate and balance queries |
//!
//! ## Accounting model
//!
//! The pool stores market totals as scaled shares (RAY); the controller stores
//! the positions (INV-ACCT-10). Token amounts convert through the market's
//! supply or borrow index. Accrual raises the indexes; bad-debt socialization
//! lowers the supply index. Protocol revenue is held as scaled supply shares,
//! so it earns the supplier rate until claimed.
//!
//! ## Security notes
//!
//! - Every mutator requires the owner through `#[only_owner]`; views are public.
//! - Cash is an accounting book, separate from the token balance. A flash loan
//!   checks the token balance after payout, after the callback and after
//!   repayment.
//! - Write paths extend the instance TTL. Every market load, views included,
//!   extends the market's params and state TTL.

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

use soroban_sdk::{
    contract, contractimpl, contractmeta, Address, Bytes, BytesN, ContractExecutable, Env, Vec,
};

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
    /// controller, to authorize.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
    }
}

#[contractimpl]
impl LiquidityPoolInterface for LiquidityPool {
    /// Creates a new asset market under `hub_id` with the given rate
    /// parameters. Initializes indexes at RAY (1.0) with zero cash, supply,
    /// and debt; panics with `AssetAlreadySupported` if the hub-asset pair
    /// already exists. Restricted to the owner.
    #[only_owner]
    fn create_market(env: Env, hub_id: u32, params: MarketParamsRaw) {
        ops::market::create(&env, hub_id, params);
    }

    /// Replaces the interest-rate model (curve, utilization cap, reserve
    /// factor) and flash-loan settings for a market. Accrues interest first so
    /// the old model applies through the current ledger, then writes the new
    /// model into market params. Restricted to the owner.
    #[only_owner]
    fn update_params(env: Env, hub_asset: HubAssetKey, model: InterestRateModel) {
        ops::market::replace_rate_model(&env, hub_asset, model);
    }

    /// Upgrades the contract WASM to `new_wasm_hash`, extending instance TTL
    /// first. Restricted to the owner.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_instance(&env);
        env.deployer()
            .update_current_contract(ContractExecutable::Wasm(new_wasm_hash));
    }

    /// Accrues, mints scaled supply shares and credits cash per entry. The
    /// controller transfers the tokens in before this call. Owner-only.
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

    /// Burns supply shares and transfers the underlying to `receiver`.
    /// `is_liquidation` skips the max-utilization check and may withhold a
    /// protocol fee. Owner-only; `actual_amount` is gross of that fee.
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

    /// Burns scaled debt up to the repay amount, credits cash with the net
    /// repay and refunds overpayment to `payer`. Owner-only.
    #[only_owner]
    fn repay(env: Env, payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        ops::run_batch(&env, actions, |env, action| {
            ops::repay::apply(env, &payer, action)
        })
    }

    /// Accrues interest for each market in `hub_assets` through the current
    /// ledger time. Commits state even with no elapsed time to reserve the write
    /// footprint, and emits its market state event. Restricted to the owner.
    #[only_owner]
    fn update_indexes(env: Env, hub_assets: Vec<HubAssetKey>) {
        ops::market::accrue(&env, hub_assets);
    }

    /// Credits cash up to the market's backing shortfall
    /// (`guards::backing_shortfall`) and transfers the excess back to `payer`.
    /// The controller transfers `amount` in before this call. Restricted to
    /// the owner; returns a [`PoolAmountMutation`] with the amount applied.
    #[only_owner]
    fn recapitalize(
        env: Env,
        hub_asset: HubAssetKey,
        payer: Address,
        amount: i128,
    ) -> PoolAmountMutation {
        ops::recapitalize::apply(&env, hub_asset, payer, amount)
    }

    /// Transfers out, invokes `execute_flash_loan` on the receiver, pulls
    /// principal plus fee back via `transfer_from`, and books the fee as
    /// protocol revenue. Returns the fee. Owner-only; requires the market to
    /// allow flash loans.
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

    /// Mints debt for `action.amount`, books the fee as protocol revenue when
    /// `charge_fee`, and sends `amount - fee` to `receiver`. Owner-only.
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

    /// Nets supply against debt on one market with no cash movement, capped by
    /// the conservative overlap of floored supply and ceiled debt. Owner-only.
    #[only_owner]
    fn net_settle(env: Env, entry: PoolNetSettleEntry) -> PoolNetSettleResult {
        renew_instance(&env);
        let (result, snapshot) = ops::net_settle::apply(&env, &entry);
        events::emit_market_state(&env, snapshot);
        result
    }

    /// Burns claimable revenue shares, debits cash and pays the owner the lesser
    /// of cash and revenue's floored token value. Returns zero when nothing is
    /// claimable. Owner-only.
    ///
    /// Decrements the snapshot `revenue` field, so that field is not a
    /// cumulative counter.
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

    /// Revenue in asset units at the **stored** index; does not accrue first.
    ///
    /// `claim_revenue` syncs before paying `min(cash, revenue)`, so the amount
    /// actually paid can be higher (pending accrual mints more shares) or
    /// lower (the cash cap binds under heavy utilization).
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

    /// Stored market params and state, without accrual.
    fn get_sync_data(env: Env, hub_asset: HubAssetKey) -> PoolSyncData {
        storage::load_sync_data(&env, &hub_asset)
    }

    /// Returns each market's indexes accrued to the current ledger time, in
    /// request order, without writing state.
    ///
    /// Runs [`simulate_update_indexes`] on each market's stored sync data.
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
