#![no_std]
// Crate-wide by necessity, not preference: #[contractimpl] generates Client
// fns mirroring the >7-arg ABI entry points (multiply is 11) at crate scope,
// where no impl- or fn-level allow can reach them. Narrowing this attribute
// re-fires the lint inside the macro expansion under `clippy -D warnings`.
#![allow(clippy::too_many_arguments)]

pub mod constants;
pub mod events;

pub use common::types;

mod account;
mod config;
mod context;
mod external;
mod governance;
mod keepers;
mod markets;
mod payments;
mod positions;
mod risk;
mod spec_hooks;
mod spoke_usage;
mod storage;
mod strategies;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../certora/controller/spec/mod.rs"]
pub mod spec;

#[cfg(feature = "testing")]
#[path = "../tests/test_support.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "../tests/entrypoints.rs"]
mod entrypoint_tests;

use common::errors::SpokeError;
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, InterestRateModel,
    LiquidationEstimate, MarketIndexRaw, MarketIndexView, MarketParamsRaw, PositionLimits,
    PositionManagerConfig, PositionMode, SeizeMode, SpokeAssetArgs, SpokeAssetConfig, SpokeConfig,
    SpokeUsageRaw,
};

use controller_interface::{ControllerAdmin, ControllerInterface};

use soroban_sdk::{
    contract, contractimpl, contractmeta, panic_with_error, Address, Bytes, BytesN, Env, Map,
    String, Vec,
};

use stellar_macros::{only_owner, when_not_paused};

use strategies::migrate_blend::MigrateBlendParams;
use strategies::multiply::MultiplyParams;
use strategies::repay_debt_with_collateral::RepayWithCollateralParams;
use strategies::swap_collateral::SwapCollateralParams;
use strategies::swap_debt::SwapDebtParams;

macro_rules! renew_then {
    ($env:ident, $body:expr) => {{
        storage::renew_controller_instance(&$env);
        $body
    }};
}

contractmeta!(key = "name", val = "Lending Controller");
contractmeta!(key = "binver", val = env!("CARGO_PKG_VERSION"));
contractmeta!(
    key = "repo",
    val = "https://github.com/xoxno/rs-lending-xlm"
);

#[contract]
pub struct Controller;

#[contractimpl]
impl Controller {
    /// Initializes governance state: sets `admin` as the contract owner, sets
    /// supply/borrow position limits to their maximum and the minimum
    /// borrow collateral floor to its default, and pauses the contract until
    /// explicitly unpaused.
    pub fn __constructor(env: Env, admin: Address) {
        governance::init(&env, &admin);
    }
}

#[contractimpl]
impl ControllerInterface for Controller {
    /// Supplies `assets` as collateral to `account_id` in spoke `spoke_id`,
    /// creating a new account when `account_id` is 0, and returns the
    /// account id.
    #[when_not_paused]
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        positions::process_supply(&env, &caller, account_id, spoke_id, &assets)
    }

    /// Borrows `borrows` against `account_id`'s collateral, sending the funds
    /// to `to` if provided or to the caller otherwise; reverts if the
    /// resulting position breaches the account's solvency limits.
    #[when_not_paused]
    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) {
        positions::process_borrow(&env, &caller, account_id, &borrows, to);
    }

    /// Withdraws `withdrawals` from `account_id`'s supplied collateral, sending
    /// the funds to `to` if provided or to the caller otherwise, and returns
    /// the amounts actually withdrawn; a zero amount for an asset withdraws
    /// the entire position.
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)> {
        positions::process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }

    /// Repays `payments` against `account_id`'s debt positions, pulling the
    /// funds from the caller.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        positions::process_repay(&env, &caller, account_id, &payments);
    }

    /// Liquidates `account_id` by having `liquidator` repay `debt_payments`
    /// and seizing collateral at a bonus scaled by the account's health
    /// factor. Permissionless — the account owner may liquidate its own
    /// account. Triggers bad-debt socialization if the account remains
    /// insolvent afterward.
    ///
    /// `seize_mode` selects delivery. `Transfer` pays the seized collateral
    /// out of pool cash. `Credit(account_id)` instead credits the seized
    /// supply shares to a controller account bound to the liquidated
    /// account's spoke, moving no tokens at all, so the liquidation can clear
    /// even when the market has no free liquidity; `Credit(0)` creates that
    /// account. Returns the receiving account id, or `0` in transfer mode.
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> u64 {
        positions::liquidation::process_liquidation(
            &env,
            &liquidator,
            account_id,
            &debt_payments,
            seize_mode,
        )
    }

    /// Socializes `account_id`'s debt into the supply index and removes the
    /// account when it is insolvent and its remaining collateral value is at
    /// or below the dust threshold; reverts otherwise.
    fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        positions::liquidation::process_clean_bad_debt(&env, &caller, account_id);
    }

    /// Flash-loans `amount` of `asset` to `receiver`, invoking its callback
    /// with `data`; the pool pulls back the principal plus fee before the
    /// call returns. `receiver` must be a deployed Wasm contract.
    #[when_not_paused]
    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    ) {
        strategies::flash_loan::process_flash_loan(&env, &caller, &asset, amount, &receiver, &data);
    }

    /// Opens or extends a leveraged position on `account_id`: borrows
    /// `debt_to_flash_loan` of `debt`, swaps it (plus any optional initial
    /// payment) into `collateral` via `swap`, and deposits the result as
    /// collateral. Returns the account id.
    #[when_not_paused]
    fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        collateral: HubAssetKey,
        debt_to_flash_loan: i128,
        debt: HubAssetKey,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(HubAssetKey, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64 {
        strategies::multiply::process_multiply(
            &env,
            &caller,
            MultiplyParams {
                account_id,
                spoke_id,
                collateral: &collateral,
                debt_to_flash_loan,
                debt: &debt,
                mode,
                swap: &swap,
                initial_payment,
                convert_swap,
            },
        )
    }

    /// Replaces `account_id`'s `existing_debt` position with `new_debt` by
    /// borrowing `amount` of `new_debt`, swapping it to `existing_debt` via
    /// `swap`, and repaying the existing position with the proceeds.
    #[when_not_paused]
    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    ) {
        strategies::swap_debt::process_swap_debt(
            &env,
            &caller,
            SwapDebtParams {
                account_id,
                existing_debt: &existing_debt,
                new_debt_amount: amount,
                new_debt: &new_debt,
                swap: &swap,
            },
        );
    }

    /// Replaces `amount` of `account_id`'s `current` collateral with `new` by
    /// withdrawing it, swapping to `new` via `swap`, and depositing the
    /// proceeds as collateral.
    #[when_not_paused]
    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    ) {
        strategies::swap_collateral::process_swap_collateral(
            &env,
            &caller,
            SwapCollateralParams {
                account_id,
                current: &current,
                from_amount: amount,
                new: &new,
                swap: &swap,
            },
        );
    }

    /// Repays `account_id`'s `debt` position using `collateral_amount` of
    /// `collateral`, netting them directly when the two assets match or
    /// swapping via `swap` otherwise. Withdraws all remaining collateral if
    /// `close_position` is set and no debt remains afterward.
    #[when_not_paused]
    fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral: HubAssetKey,
        collateral_amount: i128,
        debt: HubAssetKey,
        swap: Bytes,
        close_position: bool,
    ) {
        strategies::repay_debt_with_collateral::process_repay_debt_with_collateral(
            &env,
            &caller,
            RepayWithCollateralParams {
                account_id,
                collateral: &collateral,
                collateral_amount,
                debt: &debt,
                swap: &swap,
                close_position,
            },
        );
    }

    /// Migrates the caller's position from `blend_pool` (which must be
    /// pre-approved) into `account_id`: borrows up to `debt_caps` to repay the
    /// caller's debt on Blend, sweeps `collateral_assets` and
    /// `supply_assets` from Blend into this pool as collateral, and repays
    /// any leftover borrowed amount. Returns the account id.
    #[when_not_paused]
    fn migrate_from_blend(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        hub_id: u32,
        blend_pool: Address,
        collateral_assets: Vec<Address>,
        supply_assets: Vec<Address>,
        debt_caps: Vec<(Address, i128)>,
    ) -> u64 {
        strategies::migrate_blend::process_migrate_blend(
            &env,
            &caller,
            MigrateBlendParams {
                account_id,
                spoke_id,
                hub_id,
                blend_pool,
                collateral_assets,
                supply_assets,
                debt_caps,
            },
        )
    }

    /// Accrues the borrow and supply indexes for each hub asset in `assets`
    /// on the pool.
    #[when_not_paused]
    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        keepers::update_indexes(&env, caller, assets);
    }

    /// Claims accrued protocol revenue for each hub asset in `assets` from
    /// the pool and forwards it to the configured accumulator, returning the
    /// amount claimed per asset.
    #[when_not_paused]
    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        keepers::claim_revenue(&env, caller, assets)
    }

    /// Refreshes cached risk parameters for the supply positions of each
    /// account in `account_ids`. When `has_risks` is set, also reloads debt
    /// positions and reverts if the account's health factor falls below the
    /// update floor.
    #[when_not_paused]
    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>) {
        keepers::update_account_threshold(&env, caller, has_risks, account_ids);
    }

    /// Transfers `amount` of `hub_asset` from `payer` into the pool to cover
    /// a backing shortfall, applying only up to the shortfall and refunding
    /// any excess; returns the amount actually applied.
    fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128 {
        keepers::recapitalize(&env, payer, hub_asset, amount)
    }

    /// Extends `account_id`'s storage TTL; the caller must be the account
    /// owner.
    fn renew_account(env: Env, caller: Address, account_id: u64) {
        account::renew_account(&env, caller, account_id);
    }

    /// Grants `delegate` authority to act on behalf of `account_id`'s owner;
    /// `delegate` must already be an active, governance-approved position
    /// manager.
    #[when_not_paused]
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::add_delegate(&env, caller, account_id, delegate);
    }

    /// Revokes `delegate`'s authority over `account_id`; the caller must be
    /// the account owner.
    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::remove_delegate(&env, caller, account_id, delegate);
    }

    /// Returns whether `account_id`'s health factor is below one (WAD-scaled),
    /// meaning it is eligible for liquidation.
    fn is_liquidatable(env: Env, account_id: u64) -> bool {
        views::can_be_liquidated(&env, account_id)
    }

    /// Returns `account_id`'s health factor as a WAD-scaled ratio of
    /// liquidation-weighted collateral to debt, or `i128::MAX` when the
    /// account has no debt or does not exist.
    fn get_health_factor(env: Env, account_id: u64) -> i128 {
        views::health_factor(&env, account_id)
    }

    /// Returns the USD value (WAD) of `account_id`'s total supplied
    /// collateral.
    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::total_collateral_in_usd(&env, account_id)
    }

    /// Returns the USD value (WAD) of `account_id`'s total borrowed debt.
    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        views::total_borrow_in_usd(&env, account_id)
    }

    /// Returns `account_id`'s supplied amount of `hub_asset` in the asset's
    /// own decimals, or zero if it holds no such position.
    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::collateral_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns `account_id`'s borrowed amount of `hub_asset` in the asset's
    /// own decimals, or zero if it holds no such position.
    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::borrow_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns `account_id`'s supply and debt positions, keyed by hub asset.
    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    ) {
        views::get_account_positions(&env, account_id)
    }

    /// Returns `account_id`'s spoke id and position mode.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        views::get_account_attributes(&env, account_id)
    }

    /// Returns whether `account_id` has been created.
    fn account_exists(env: Env, account_id: u64) -> bool {
        views::account_exists(&env, account_id)
    }

    /// Simulates liquidating `account_id` with `debt_payments` under
    /// `seize_mode` without changing state. Returns the collateral that would
    /// be seized, the protocol fees, any refunds, the maximum payable debt
    /// (WAD), and the applicable bonus rate (BPS).
    ///
    /// Seized and fee amounts are reported in the units the chosen mode moves:
    /// asset units for `Transfer`, RAY-scaled supply shares for `Credit`.
    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> LiquidationEstimate {
        views::liquidation_estimations_detailed(&env, account_id, &debt_payments, seize_mode)
    }

    /// Returns `account_id`'s liquidation-threshold-weighted collateral value
    /// (WAD), the ceiling on collateral seizable during liquidation.
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        views::liquidation_collateral_available(&env, account_id)
    }

    /// Returns `account_id`'s LTV-weighted collateral value (WAD), the
    /// ceiling on its borrowing power.
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::ltv_collateral_in_usd(&env, account_id)
    }

    /// Returns the address of the deployed liquidity pool contract.
    fn get_pool_address(env: Env) -> Address {
        views::get_pool_address(&env)
    }

    /// Returns the current supply and borrow indexes (RAY) for `hub_asset`.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw {
        let mut cache = context::Cache::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&hub_asset))
    }

    /// Returns the supply/borrow indexes and current oracle price status for
    /// each hub asset in `hub_assets`; reverts if more assets than the
    /// configured maximum are requested.
    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView> {
        views::get_all_market_indexes_detailed(&env, &hub_assets)
    }

    /// Returns the configuration of spoke `spoke_id`.
    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    /// Returns the configuration of `hub_asset` within spoke `spoke_id`;
    /// panics if the asset is not listed there.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig {
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, SpokeError::AssetNotInSpoke))
    }

    /// Returns the current supplied and borrowed usage of `hub_asset` within
    /// spoke `spoke_id`, or a zeroed value if none is recorded.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw {
        storage::get_spoke_usage(&env, spoke_id, &hub_asset).unwrap_or_default()
    }

    /// Returns the configured price aggregator contract address.
    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    /// Returns the minimum collateral value (WAD) required to open a new
    /// borrow position.
    fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    /// Returns whether `pool` is approved as a Blend migration source.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        config::registry::is_blend_pool_approved(&env, pool)
    }
}

#[contractimpl]
impl ControllerAdmin for Controller {
    /// Sets the swap aggregator contract address used by strategy swaps.
    /// Restricted to the owner.
    #[only_owner]
    fn set_swap_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_swap_aggregator(&env, addr))
    }

    /// Sets the price aggregator contract address used for oracle lookups.
    /// Restricted to the owner.
    #[only_owner]
    fn set_price_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_price_aggregator(&env, addr))
    }

    /// Sets the accumulator address that receives claimed protocol revenue.
    /// Restricted to the owner.
    #[only_owner]
    fn set_accumulator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_accumulator(&env, addr))
    }

    /// Sets the maximum number of concurrent supply and borrow positions an
    /// account may hold. Restricted to the owner. Both bounds must be
    /// within the configured maximum.
    #[only_owner]
    fn set_position_limits(env: Env, limits: PositionLimits) {
        renew_then!(env, config::registry::set_position_limits(&env, limits))
    }

    /// Sets the minimum collateral value (WAD) required to open a new
    /// borrow position. Restricted to the owner. `floor_wad` must be
    /// non-negative.
    #[only_owner]
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        renew_then!(
            env,
            config::registry::set_min_borrow_collateral_usd(&env, floor_wad)
        )
    }

    /// Activates or deactivates `manager` as a position manager eligible to
    /// be granted delegate access on accounts. Restricted to the owner.
    #[only_owner]
    fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        renew_then!(
            env,
            storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active })
        )
    }

    /// Approves `pool` as a Blend migration source. Restricted to the
    /// owner.
    #[only_owner]
    fn approve_blend_pool(env: Env, pool: Address) {
        renew_then!(
            env,
            config::registry::set_blend_pool_approval(&env, pool, true)
        )
    }

    /// Revokes `pool` as an approved Blend migration source. Restricted to
    /// the owner.
    #[only_owner]
    fn revoke_blend_pool(env: Env, pool: Address) {
        renew_then!(
            env,
            config::registry::set_blend_pool_approval(&env, pool, false)
        )
    }

    /// Creates a new active hub and returns its id. Restricted to the
    /// owner.
    #[only_owner]
    fn create_hub(env: Env) -> u32 {
        renew_then!(env, config::spoke::create_hub(&env))
    }

    /// Creates a new spoke with the default liquidation curve and returns
    /// its id. Restricted to the owner.
    #[only_owner]
    fn add_spoke(env: Env) -> u32 {
        renew_then!(env, config::spoke::add_spoke(&env))
    }

    /// Marks spoke `id` as deprecated, blocking new positions in it.
    /// Restricted to the owner. Reverts if it is already deprecated.
    #[only_owner]
    fn remove_spoke(env: Env, id: u32) {
        renew_then!(env, config::spoke::remove_spoke(&env, id))
    }

    /// Sets spoke `id`'s liquidation curve: the target health factor, the
    /// health factor at which the bonus is maximal, and the bonus scaling
    /// factor (BPS). Restricted to the owner.
    #[only_owner]
    fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    ) {
        renew_then!(
            env,
            config::spoke::set_spoke_liquidation_curve(
                &env,
                id,
                target_hf_wad,
                hf_for_max_bonus_wad,
                liquidation_bonus_factor_bps,
            )
        )
    }

    /// Lists a new asset in a spoke with its risk parameters and caps,
    /// validating them against the asset's pool decimals. Restricted to the
    /// owner. Reverts if the spoke is deprecated or the asset is already
    /// listed.
    #[only_owner]
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::add_asset_to_spoke(&env, &input))
    }

    /// Updates an already-listed spoke asset's risk parameters and caps,
    /// revalidating them against the asset's pool decimals. Restricted to
    /// the owner. Reverts if the asset is not listed in the spoke.
    #[only_owner]
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::edit_asset_in_spoke(&env, &input))
    }

    /// Sets the paused, frozen, and no-seize flags for `hub_asset` within
    /// spoke `spoke_id`. Restricted to the owner. Flags can only be tightened
    /// to true, not relaxed, through this function.
    #[only_owner]
    fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
        no_seize: bool,
    ) {
        renew_then!(
            env,
            config::asset::set_spoke_asset_flags(
                &env, spoke_id, hub_asset, paused, frozen, no_seize
            )
        )
    }

    /// Removes `hub_asset` from spoke `spoke_id`. Restricted to the owner.
    /// Reverts if the asset is not listed or still has outstanding supplied
    /// or borrowed usage.
    #[only_owner]
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        renew_then!(
            env,
            config::asset::remove_asset_from_spoke(&env, hub_asset, spoke_id)
        )
    }

    /// Deploys the liquidity pool contract from `wasm_hash` and records its
    /// address. Restricted to the owner. Reverts if a pool has already been
    /// deployed.
    #[only_owner]
    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address {
        renew_then!(env, markets::deploy_pool(&env, wasm_hash))
    }

    /// Deploys the position-NFT contract that anchors account ownership.
    /// One-shot; restricted to the owner.
    #[only_owner]
    fn deploy_position_nft(
        env: Env,
        wasm_hash: BytesN<32>,
        uri: String,
        name: String,
        symbol: String,
    ) -> Address {
        renew_then!(
            env,
            markets::deploy_position_nft(&env, wasm_hash, uri, name, symbol)
        )
    }

    /// Creates a new market for `asset` under hub `hub_id` on the pool
    /// using `params`. Restricted to the owner. Reverts if the hub is
    /// inactive or `params.asset_id` does not match `asset`.
    #[only_owner]
    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address {
        renew_then!(
            env,
            markets::create_liquidity_pool(&env, hub_id, asset, params)
        )
    }

    /// Accrues `hub_asset`'s indexes, then updates its interest rate model
    /// to `params` on the pool. Restricted to the owner.
    #[only_owner]
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel) {
        renew_then!(
            env,
            markets::upgrade_liquidity_pool_params(&env, &hub_asset, &params)
        )
    }

    /// Upgrades the liquidity pool contract to `new_wasm_hash`. Restricted
    /// to the owner.
    #[only_owner]
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, markets::upgrade_pool(&env, new_wasm_hash))
    }

    /// Upgrades the position-NFT contract's Wasm bytecode to
    /// `new_wasm_hash`. Restricted to the owner.
    #[only_owner]
    fn upgrade_position_nft(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, markets::upgrade_position_nft(&env, new_wasm_hash))
    }

    /// Force-socializes `account_id`'s debt into the supply index when the
    /// account is insolvent. Restricted to the owner. Bypasses the
    /// dust-collateral cap that gates the permissionless cleanup.
    #[only_owner]
    fn force_socialize_bad_debt(env: Env, account_id: u64) {
        renew_then!(
            env,
            positions::liquidation::process_force_socialize_bad_debt(&env, account_id)
        )
    }

    /// Pauses the contract. Restricted to the owner.
    #[only_owner]
    fn pause(env: Env) {
        renew_then!(env, governance::pause(&env))
    }

    /// Unpauses the contract. Restricted to the owner.
    #[only_owner]
    fn unpause(env: Env) {
        renew_then!(env, governance::unpause(&env))
    }

    /// Pauses the contract if it is not already paused, then upgrades it to
    /// `new_wasm_hash`. Restricted to the owner.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, governance::upgrade(&env, &new_wasm_hash))
    }

    /// Sets the stored app version to `new_version`. Restricted to the
    /// owner. Reverts unless it is greater than the current version.
    #[only_owner]
    fn migrate(env: Env, new_version: u32) {
        renew_then!(env, governance::migrate(&env, new_version))
    }

    /// Returns the contract's current app version.
    fn get_app_version(env: Env) -> u32 {
        governance::get_app_version(&env)
    }

    /// Begins a two-step transfer of ownership to `new_owner`. Restricted
    /// to the owner. The pending grant expires at ledger
    /// `live_until_ledger` unless accepted.
    #[only_owner]
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        renew_then!(
            env,
            governance::transfer_ownership(&env, &new_owner, live_until_ledger)
        )
    }

    /// Completes a pending ownership transfer, making the caller the new
    /// owner.
    fn accept_ownership(env: Env) {
        governance::accept_ownership(&env);
    }
}
