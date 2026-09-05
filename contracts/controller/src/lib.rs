#![no_std]
// #[contractimpl] generates client functions at crate scope for the >7-argument
// ABI entry points, so this lint allowance cannot be limited to their impl.
#![allow(clippy::too_many_arguments)]

pub mod constants;
pub mod events;

pub use common::types;

mod account;
mod config;
mod context;
mod external;
mod governance;
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

use strategies::flash_position::FlashPositionParams;
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
    /// Sets `admin` as owner, initializes maximum position limits, the default
    /// borrow collateral floor and app version, and leaves the contract paused.
    pub fn __constructor(env: Env, admin: Address) {
        governance::init(&env, &admin);
    }
}

#[contractimpl]
impl ControllerInterface for Controller {
    /// Supplies `assets` as collateral and returns the account id; `account_id = 0`
    /// creates an account in `spoke_id`. Third parties may only top up existing
    /// supply positions; owners and delegates may add assets.
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

    /// Borrows against `account_id`'s collateral, paying `to` or the caller.
    /// Requires owner or delegate authorization and post-borrow solvency.
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

    /// Withdraws collateral to `to` or the caller and returns actual amounts in
    /// asset units. Zero withdraws an asset's full position. Requires owner or
    /// delegate authorization and post-withdrawal solvency.
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)> {
        positions::process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }

    /// Repays `account_id`'s debt using measured payments from the caller.
    /// Anyone may repay; excess payments are refunded to the caller.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        positions::process_repay(&env, &caller, account_id, &payments);
    }

    /// Repays debt and seizes collateral at a health-factor-based bonus.
    /// Permissionless, including self-liquidation; requires liquidator authorization.
    /// Residual bad debt is socialized only at or below the collateral dust cap.
    ///
    /// `Transfer` pays pool cash and returns `0`. `Credit(id)` moves net supply
    /// shares to a different, authorized Normal-mode account on the same spoke;
    /// `Credit(0)` creates one. Credit mode needs no free collateral liquidity
    /// and returns the receiving account id.
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

    /// Socializes insolvent debt into the supply index and removes the account
    /// when remaining collateral is at or below the dust cap. Permissionless;
    /// requires caller authorization.
    fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        positions::liquidation::process_clean_bad_debt(&env, &caller, account_id);
    }

    /// Flash-loans `amount` of `asset` to a deployed Wasm `receiver`, invoking
    /// its callback with `data`. The pool recovers principal plus fee before return.
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

    /// Mints `amount` of `debt` without a flash fee, forwards measured receipts
    /// and invokes the Wasm receiver's `execute_flash_position` callback.
    /// `collaterals` sets minimum controller-balance increases to deposit;
    /// listed `refund_assets` balance increases return to the caller.
    /// Returns the solvent account's id; `account_id = 0` creates it.
    #[when_not_paused]
    fn flash_position(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        mode: PositionMode,
        debt: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
        collaterals: Vec<(HubAssetKey, i128)>,
        refund_assets: Vec<Address>,
    ) -> u64 {
        strategies::flash_position::process_flash_position(
            &env,
            &caller,
            FlashPositionParams {
                account_id,
                spoke_id,
                mode,
                debt: &debt,
                amount,
                receiver: &receiver,
                data: &data,
                collaterals: &collaterals,
                refund_assets: &refund_assets,
            },
        )
    }

    /// Borrows `debt_to_flash_loan`, swaps into `collateral` and deposits the
    /// proceeds. An `initial_payment` in collateral joins the deposit; one in debt
    /// joins `swap`; a third asset requires `convert_swap` or reverts with
    /// `ConvertStepsRequired`. Returns the account id; `account_id = 0` creates it.
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

    /// Borrows `amount` of `new_debt`, converts it to `existing_debt` via `swap`
    /// and repays with the proceeds. Requires owner or delegate authorization.
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

    /// Withdraws `amount` of `current`, converts it to `new` via `swap` and
    /// redeposits the proceeds. Requires owner or delegate authorization.
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

    /// Repays `debt` from `collateral`, netting directly for the same hub asset
    /// (`swap` must be empty) or converting otherwise. `close_position` withdraws
    /// all remaining collateral to the caller, reverting with
    /// `CannotCloseWithRemainingDebt` if any debt remains.
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

    /// Migrates the caller's position from an approved `blend_pool`: borrows
    /// `debt_caps`, repays Blend and unused borrowing, then deposits withdrawn
    /// `collateral_assets` and `supply_assets`. Returns the account id;
    /// `account_id = 0` creates it.
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

    /// Accrues pool borrow and supply indexes for `assets`.
    #[when_not_paused]
    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        markets::update_indexes(&env, caller, assets);
    }

    /// Claims pool revenue and forwards measured receipts to the accumulator.
    /// Returns those amounts in asset units, in input order.
    #[when_not_paused]
    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        markets::claim_revenue(&env, caller, assets)
    }

    /// Refreshes supply LTV snapshots. With `has_risks`, also refreshes gated
    /// liquidation parameters and requires a final health factor of at least
    /// 1.05 WAD. Permissionless; requires caller authorization.
    #[when_not_paused]
    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>) {
        risk::params::update_account_threshold(&env, caller, has_risks, account_ids);
    }

    /// Covers a pool backing shortfall using measured receipts from `payer`.
    /// Refunds excess and returns the amount applied in asset units.
    fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128 {
        markets::recapitalize(&env, payer, hub_asset, amount)
    }

    /// Extends account storage TTL. Requires account-owner authorization.
    fn renew_account(env: Env, caller: Address, account_id: u64) {
        account::renew_account(&env, caller, account_id);
    }

    /// Grants account access to an active, governance-approved position manager.
    /// Requires account-owner authorization.
    #[when_not_paused]
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::add_delegate(&env, caller, account_id, delegate);
    }

    /// Revokes delegate access. Requires account-owner authorization.
    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::remove_delegate(&env, caller, account_id, delegate);
    }

    /// Returns whether the account's health factor is below 1.0 WAD.
    fn is_liquidatable(env: Env, account_id: u64) -> bool {
        views::can_be_liquidated(&env, account_id)
    }

    /// Returns liquidation-weighted collateral divided by debt, in WAD;
    /// `i128::MAX` if the account has no debt or does not exist.
    fn get_health_factor(env: Env, account_id: u64) -> i128 {
        views::health_factor(&env, account_id)
    }

    /// Returns total supplied collateral in USD (WAD).
    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::total_collateral_in_usd(&env, account_id)
    }

    /// Returns total borrowed debt in USD (WAD).
    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        views::total_borrow_in_usd(&env, account_id)
    }

    /// Returns supplied `hub_asset` in asset units, or zero without a position.
    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::collateral_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns borrowed `hub_asset` in asset units, or zero without a position.
    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::borrow_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns supply and debt positions keyed by hub asset, or empty maps
    /// if the account does not exist.
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

    /// Returns whether the account has stored metadata.
    fn account_exists(env: Env, account_id: u64) -> bool {
        views::account_exists(&env, account_id)
    }

    /// Estimates gross seizure, protocol fees, refunds, maximum payable debt
    /// in USD (WAD), and bonus (BPS). Seizure and fees use asset units for
    /// `Transfer`, RAY-scaled supply shares for `Credit`.
    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> LiquidationEstimate {
        views::liquidation_estimations_detailed(&env, account_id, &debt_payments, seize_mode)
    }

    /// Returns liquidation-threshold-weighted collateral in USD (WAD).
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        views::liquidation_collateral_available(&env, account_id)
    }

    /// Returns LTV-weighted collateral in USD (WAD), the borrowing limit.
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::ltv_collateral_in_usd(&env, account_id)
    }

    /// Returns the deployed liquidity pool address.
    fn get_pool_address(env: Env) -> Address {
        storage::get_pool(&env)
    }

    /// Returns the current supply and borrow indexes (RAY) for `hub_asset`.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw {
        let mut cache = context::Context::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&hub_asset))
    }

    /// Returns supply/borrow indexes (RAY) and oracle price status for each
    /// hub asset. Reverts above `MAX_VIEW_INPUTS`.
    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView> {
        views::get_all_market_indexes_detailed(&env, &hub_assets)
    }

    /// Returns the configuration of spoke `spoke_id`.
    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    /// Returns the spoke asset's configuration; reverts if it is not listed.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig {
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, SpokeError::AssetNotInSpoke))
    }

    /// Returns spoke supply and borrow usage, or zeroed usage if unrecorded.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw {
        storage::get_spoke_usage(&env, spoke_id, &hub_asset).unwrap_or_default()
    }

    /// Returns the configured price aggregator contract address.
    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    /// Returns the minimum collateral in USD (WAD) for a new borrow position.
    fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    /// Returns whether `pool` is approved as a Blend migration source.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        storage::is_blend_pool_approved(&env, &pool)
    }
}

#[contractimpl]
impl ControllerAdmin for Controller {
    /// Sets the strategy swap aggregator address. Owner-only.
    #[only_owner]
    fn set_swap_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_swap_aggregator(&env, addr))
    }

    /// Sets the oracle price aggregator address. Owner-only.
    #[only_owner]
    fn set_price_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_price_aggregator(&env, addr))
    }

    /// Sets the recipient of claimed protocol revenue. Owner-only.
    #[only_owner]
    fn set_accumulator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_accumulator(&env, addr))
    }

    /// Sets per-account supply and borrow position limits in
    /// `1..=POSITION_LIMIT_MAX`. Owner-only.
    #[only_owner]
    fn set_position_limits(env: Env, limits: PositionLimits) {
        renew_then!(env, config::registry::set_position_limits(&env, limits))
    }

    /// Sets the non-negative collateral floor in USD (WAD) for new borrow
    /// positions. Owner-only.
    #[only_owner]
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        renew_then!(
            env,
            config::registry::set_min_borrow_collateral_usd(&env, floor_wad)
        )
    }

    /// Activates or deactivates a manager's eligibility for account delegation.
    /// Owner-only.
    #[only_owner]
    fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        renew_then!(
            env,
            storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active })
        )
    }

    /// Approves a Blend migration source. Owner-only.
    #[only_owner]
    fn approve_blend_pool(env: Env, pool: Address) {
        renew_then!(
            env,
            config::registry::set_blend_pool_approval(&env, pool, true)
        )
    }

    /// Revokes a Blend migration source. Owner-only.
    #[only_owner]
    fn revoke_blend_pool(env: Env, pool: Address) {
        renew_then!(
            env,
            config::registry::set_blend_pool_approval(&env, pool, false)
        )
    }

    /// Creates an active hub and returns its id. Owner-only.
    #[only_owner]
    fn create_hub(env: Env) -> u32 {
        renew_then!(env, config::spoke::create_hub(&env))
    }

    /// Creates a spoke with the default liquidation curve and returns its id.
    /// Owner-only.
    #[only_owner]
    fn add_spoke(env: Env) -> u32 {
        renew_then!(env, config::spoke::add_spoke(&env))
    }

    /// Deprecates a spoke, blocking ordinary position entry. Owner-only;
    /// reverts if already deprecated. Repayment and liquidation remain available.
    #[only_owner]
    fn remove_spoke(env: Env, id: u32) {
        renew_then!(env, config::spoke::remove_spoke(&env, id))
    }

    /// Sets target and max-bonus health factors (WAD) and the bonus factor
    /// (BPS) for a spoke. Owner-only.
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

    /// Lists a spoke asset after validating risk parameters and caps against
    /// pool decimals. Owner-only; rejects deprecated spokes and duplicate listings.
    #[only_owner]
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::add_asset_to_spoke(&env, &input))
    }

    /// Updates a listed spoke asset's risk parameters and caps, validating
    /// against pool decimals. Owner-only; rejects unlisted assets.
    #[only_owner]
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::edit_asset_in_spoke(&env, &input))
    }

    /// Tightens a spoke asset's paused, frozen and no-seize flags to true.
    /// Cannot clear flags. Owner-only.
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

    /// Removes a listed spoke asset with no supply or borrow usage. Owner-only.
    #[only_owner]
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        renew_then!(
            env,
            config::asset::remove_asset_from_spoke(&env, hub_asset, spoke_id)
        )
    }

    /// Deploys the pool from `wasm_hash`, stores and returns its address.
    /// Owner-only; rejects redeployment.
    #[only_owner]
    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address {
        renew_then!(env, markets::deploy_pool(&env, wasm_hash))
    }

    /// Deploys the account-ownership NFT and returns its address.
    /// Owner-only; rejects redeployment.
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

    /// Creates a pool market and returns the pool address. Owner-only;
    /// requires an active hub and `params.asset_id == asset`.
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

    /// Accrues market indexes before replacing the interest-rate model.
    /// Owner-only.
    #[only_owner]
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel) {
        renew_then!(
            env,
            markets::upgrade_liquidity_pool_params(&env, &hub_asset, &params)
        )
    }

    /// Upgrades the pool to `new_wasm_hash`. Owner-only.
    #[only_owner]
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, markets::upgrade_pool(&env, new_wasm_hash))
    }

    /// Upgrades the position NFT to `new_wasm_hash`. Owner-only.
    #[only_owner]
    fn upgrade_position_nft(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, markets::upgrade_position_nft(&env, new_wasm_hash))
    }

    /// Socializes insolvent debt into the supply index and removes the account,
    /// bypassing the permissionless collateral dust cap. Owner-only.
    #[only_owner]
    fn force_socialize_bad_debt(env: Env, account_id: u64) {
        renew_then!(
            env,
            positions::liquidation::process_force_socialize_bad_debt(&env, account_id)
        )
    }

    /// Pauses guarded operations. Owner-only. Repayment, withdrawal and
    /// liquidation remain available subject to their asset-level gates.
    #[only_owner]
    fn pause(env: Env) {
        renew_then!(env, governance::pause(&env))
    }

    /// Unpauses guarded operations. Owner-only.
    #[only_owner]
    fn unpause(env: Env) {
        renew_then!(env, governance::unpause(&env))
    }

    /// Pauses the controller if needed, then upgrades to `new_wasm_hash`.
    /// Owner-only.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, governance::upgrade(&env, &new_wasm_hash))
    }

    /// Sets an app version strictly greater than the current version. Owner-only.
    #[only_owner]
    fn migrate(env: Env, new_version: u32) {
        renew_then!(env, governance::migrate(&env, new_version))
    }

    /// Returns the current app version.
    fn get_app_version(env: Env) -> u32 {
        governance::get_app_version(&env)
    }

    /// Starts an owner-only, two-step transfer to `new_owner`; the pending
    /// grant expires at `live_until_ledger` unless accepted.
    #[only_owner]
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        renew_then!(
            env,
            governance::transfer_ownership(&env, &new_owner, live_until_ledger)
        )
    }

    /// Completes the pending ownership transfer with the new owner's authorization.
    fn accept_ownership(env: Env) {
        governance::accept_ownership(&env);
    }
}
