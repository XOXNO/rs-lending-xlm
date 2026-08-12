//! Entry point for the Lending Controller Soroban contract. Defines the `Controller`
//! contract type and its `#[contractimpl]` implementations of `ControllerInterface`
//! (user-facing supply/borrow/repay/withdraw, liquidation, leverage-strategy, and view
//! operations) and `ControllerAdmin` (owner-gated configuration and governance
//! operations). Each method is a thin dispatcher that delegates to the corresponding
//! submodule.
#![no_std]
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
mod spoke;
mod storage;
mod strategies;
mod views;

#[cfg(feature = "certora")]
#[path = "../../../certora/controller/spec/mod.rs"]
pub mod spec;

#[cfg(feature = "testing")]
#[path = "../tests/test_support.rs"]
pub mod test_support;

use common::errors::SpokeError;
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, InterestRateModel,
    LiquidationEstimate, MarketIndexRaw, MarketIndexView, MarketParamsRaw, PositionLimits,
    PositionManagerConfig, PositionMode, SpokeAssetArgs, SpokeAssetConfig, SpokeConfig,
    SpokeUsageRaw,
};

use controller_interface::{ControllerAdmin, ControllerInterface};

use soroban_sdk::{
    contract, contractimpl, contractmeta, panic_with_error, Address, Bytes, BytesN, Env, Map, Vec,
};

use stellar_macros::{only_owner, when_not_paused};

use strategies::migrate_blend::MigrateBlendParams;
use strategies::multiply::MultiplyParams;
use strategies::repay_debt_with_collateral::RepayWithCollateralParams;
use strategies::swap_collateral::SwapCollateralParams;
use strategies::swap_debt::SwapDebtParams;

/// Renews the controller instance TTL, then evaluates the body.
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

/// The Lending Controller contract type. Implements `ControllerInterface` for
/// user-facing lending, strategy, and view operations, and `ControllerAdmin` for
/// owner-gated configuration and governance.
#[contract]
pub struct Controller;

#[contractimpl]
impl Controller {
    /// Constructor invoked on contract deployment. Sets `admin` as the contract
    /// owner, applies default position limits and minimum borrow collateral, stores
    /// the initial app version, and pauses the contract.
    pub fn __constructor(env: Env, admin: Address) {
        governance::access::init(&env, &admin);
    }
}

#[contractimpl]
impl ControllerInterface for Controller {
    /// Deposits `assets` into the pool as supply collateral for `account_id` under
    /// `spoke_id`, creating the account first if `account_id` is zero. Once the
    /// account already exists, non-owners may only top up assets they already hold
    /// unless the caller is both an active protocol position manager and listed among
    /// the account's delegates. Returns the account ID.
    #[when_not_paused]
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        positions::supply::process_supply(&env, &caller, account_id, spoke_id, &assets)
    }

    /// Borrows `borrows` from the pool against `account_id`'s collateral, crediting
    /// the proceeds to `to` (or `caller` if omitted). Requires the caller to be the
    /// account owner, or an active protocol position manager listed among the
    /// account's delegates, and enforces post-borrow solvency.
    #[when_not_paused]
    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) {
        positions::borrow::process_borrow(&env, &caller, account_id, &borrows, to);
    }

    /// Withdraws `withdrawals` from `account_id`'s supply positions, paying out to
    /// `to` (or `caller` if omitted). A zero amount for an asset withdraws that
    /// position's full balance. Requires the caller to be the account owner, or an
    /// active protocol position manager listed among the account's delegates.
    /// Returns the amount actually paid out per asset.
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)> {
        positions::withdraw::process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }

    /// Repays `payments` against `account_id`'s debt positions, pulling the payment
    /// tokens from `caller`. Any caller may repay on the account's behalf.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        positions::repay::process_repay(&env, &caller, account_id, &payments);
    }

    /// Liquidates `account_id` by repaying `debt_payments` and seizing a matching
    /// share of collateral plus a bonus, credited to `liquidator`. Requires
    /// `liquidator`'s authorization, rejects self-liquidation, and requires that no
    /// flash loan is currently in progress. Socializes any bad debt left within the
    /// dust-capped threshold after the liquidation.
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) {
        positions::liquidation::process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    /// Socializes `account_id`'s bad debt, provided its outstanding debt exceeds
    /// collateral and the collateral is within the dust-capped bad-debt threshold.
    /// Requires `caller`'s authorization and that no flash loan is currently in
    /// progress. Panics if the account has no debt positions or does not qualify.
    fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        positions::liquidation::process_clean_bad_debt(&env, &caller, account_id);
    }

    /// Lends `amount` of `asset` to `receiver` for the duration of a single
    /// transaction, invoking `receiver` with `data` and requiring full repayment
    /// plus a fee before the call returns. Requires `caller`'s authorization, that no
    /// flash loan is already in progress, and that `receiver` is a WASM contract.
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

    /// Opens or extends a leveraged position for `account_id` under `spoke_id`:
    /// borrows `debt_to_flash_loan` units of `debt` via the pool strategy-borrow
    /// path (not a flash loan), swaps the proceeds (plus any optional
    /// `initial_payment`, converted via `convert_swap` if it is in a third asset) into
    /// `collateral` using `swap`, and supplies the total as collateral. Requires
    /// `caller`'s authorization, that no flash loan is currently in progress, and
    /// that for existing accounts `caller` is the owner or an active protocol
    /// position manager listed among the account's delegates (new accounts take
    /// `caller` as owner). Returns the account ID.
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

    /// Replaces part of `account_id`'s `existing_debt` with `new_debt`: borrows
    /// `amount` of `new_debt`, swaps the proceeds into `existing_debt` via `swap`,
    /// and repays `existing_debt` with the result. Requires `caller`'s authorization,
    /// that `caller` is the account owner or an active protocol position manager
    /// listed among the account's delegates, that `existing_debt` and `new_debt`
    /// differ, and that no flash loan is currently in progress.
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

    /// Replaces `amount` of `account_id`'s `current` supply collateral with `new`:
    /// withdraws `amount` of `current`, swaps it into `new` via `swap`, and supplies
    /// the result as the `new` collateral. Requires `caller`'s authorization, that
    /// `caller` is the account owner or an active protocol position manager listed
    /// among the account's delegates, that `current` and `new` differ, and that no
    /// flash loan is currently in progress.
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

    /// Repays `account_id`'s `debt` using `collateral_amount` of `collateral`. If
    /// `collateral` and `debt` are the same asset, nets the amount directly against
    /// the debt (requires an empty `swap`); otherwise withdraws the collateral, swaps
    /// it into `debt` via `swap`, and repays with the proceeds. If `close_position` is
    /// set, also withdraws any remaining collateral once all debt is repaid. Requires
    /// `caller`'s authorization, that `caller` is the account owner or an active
    /// protocol position manager listed among the account's delegates, and that no
    /// flash loan is currently in progress.
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

    /// Migrates a position from `blend_pool` into `account_id` under `spoke_id`:
    /// repays the account's debt on `blend_pool` for each `(asset, max)` pair in
    /// `debt_caps` using controller-borrowed hub debt, then withdraws
    /// `collateral_assets` and `supply_assets` from `blend_pool` and deposits them as
    /// supply under `hub_id`. Requires `caller`'s authorization, that for existing
    /// accounts `caller` is the owner or an active protocol position manager listed
    /// among the account's delegates (new accounts take `caller` as owner), that
    /// `hub_id` is active, and that no flash loan is currently in progress. Returns
    /// the account ID.
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

    /// Refreshes the pool's interest indexes for each hub asset in `assets`.
    /// Requires `caller`'s authorization and that no flash loan is currently in
    /// progress.
    #[when_not_paused]
    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        keepers::update_indexes(&env, caller, assets);
    }

    /// Claims accrued revenue from the pool for each hub asset in `assets` and
    /// forwards it to the revenue accumulator. Requires `caller`'s authorization and
    /// that no flash loan is currently in progress. Returns the claimed amount per
    /// asset, in the same order as `assets`. Panics if no revenue accumulator is set.
    #[when_not_paused]
    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        keepers::claim_revenue(&env, caller, assets)
    }

    /// Refreshes supply-position risk parameters for each account in `account_ids`
    /// against current spoke configuration. When `has_risks` is set, refreshes the
    /// full liquidation tuple (not LTV-only), loads debt only to recompute health
    /// factor, and asserts HF meets the minimum threshold. Requires `caller`'s
    /// authorization and that no flash loan is currently in progress.
    #[when_not_paused]
    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>) {
        keepers::update_account_threshold(&env, caller, has_risks, account_ids);
    }

    /// Recapitalizes the pool's `hub_asset` reserve with `amount` transferred from
    /// `payer`, crediting only the amount actually received. Requires `payer`'s
    /// authorization and that no flash loan is currently in progress. Returns the
    /// credited amount.
    fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128 {
        keepers::recapitalize(&env, payer, hub_asset, amount)
    }

    /// Renews `account_id`'s storage TTL. Requires `caller`'s authorization and
    /// ownership of the account.
    fn renew_account(env: Env, caller: Address, account_id: u64) {
        account::renew_account(&env, caller, account_id);
    }

    /// Grants `delegate` as a delegate of `account_id`. Requires `caller`'s
    /// authorization and ownership of the account.
    #[when_not_paused]
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::add_delegate(&env, caller, account_id, delegate);
    }

    /// Revokes `delegate` as a delegate of `account_id`. Requires `caller`'s
    /// authorization and ownership of the account.
    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::remove_delegate(&env, caller, account_id, delegate);
    }

    /// Returns whether `account_id`'s health factor is below the WAD-scaled
    /// liquidation threshold (`1.0`).
    fn is_liquidatable(env: Env, account_id: u64) -> bool {
        views::can_be_liquidated(&env, account_id)
    }

    /// Returns `account_id`'s current health factor as a raw WAD value, or
    /// `i128::MAX` if the account does not exist or carries no debt.
    fn get_health_factor(env: Env, account_id: u64) -> i128 {
        views::health_factor(&env, account_id)
    }

    /// Returns the USD value (raw WAD) of `account_id`'s total supply collateral, or
    /// zero if the account has no supply positions.
    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::total_collateral_in_usd(&env, account_id)
    }

    /// Returns the USD value (raw WAD) of `account_id`'s total debt, or zero if the
    /// account has no debt positions.
    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        views::total_borrow_in_usd(&env, account_id)
    }

    /// Returns `account_id`'s current supply balance for `hub_asset`, in asset
    /// units, or zero if it holds no such position.
    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::collateral_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns `account_id`'s current debt balance for `hub_asset`, in asset units,
    /// or zero if it holds no such position.
    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::borrow_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns `account_id`'s raw supply and debt positions, keyed by hub asset.
    /// Returns two empty maps if the account does not exist.
    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    ) {
        views::get_account_positions(&env, account_id)
    }

    /// Returns `account_id`'s account attributes — its spoke and position mode,
    /// projected from the stored account metadata (which omits the owner address).
    /// Panics if the account does not exist.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        views::get_account_attributes(&env, account_id)
    }

    /// Returns whether `account_id` has stored account metadata.
    fn account_exists(env: Env, account_id: u64) -> bool {
        views::account_exists(&env, account_id)
    }

    /// Simulates liquidating `account_id` with `debt_payments` without mutating
    /// state, returning the collateral that would be seized, the protocol fees, any
    /// refunds, the maximum USD payment accepted, and the applicable bonus rate.
    /// Panics if `debt_payments` exceeds the maximum view-input length.
    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate {
        views::liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    /// Returns `account_id`'s liquidation-threshold-weighted collateral value (raw
    /// WAD), or zero if the account does not exist.
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        views::liquidation_collateral_available(&env, account_id)
    }

    /// Returns the USD value (raw WAD) of `account_id`'s supply collateral weighted
    /// by loan-to-value ratio, after restamping any listed positions. Zero if the
    /// account does not exist.
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::ltv_collateral_in_usd(&env, account_id)
    }

    /// Returns the address of the deployed liquidity-pool contract.
    fn get_pool_address(env: Env) -> Address {
        views::get_pool_address(&env)
    }

    /// Returns the current cached supply and borrow indexes for `hub_asset`.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw {
        let mut cache = context::Cache::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&hub_asset))
    }

    /// Returns detailed market data (indexes and price status) for each asset in
    /// `hub_assets`. Panics if `hub_assets` exceeds the maximum view-input length.
    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView> {
        views::get_all_market_indexes_detailed(&env, &hub_assets)
    }

    /// Returns the stored configuration for `spoke_id`. Panics if no spoke exists
    /// for that ID.
    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    /// Returns the stored configuration for `hub_asset` within `spoke_id`. Panics if
    /// the asset is not listed in the spoke.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig {
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, SpokeError::AssetNotInSpoke))
    }

    /// Returns the stored usage totals for `hub_asset` within `spoke_id`, or the
    /// default (zero) usage if none is stored.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw {
        storage::get_spoke_usage(&env, spoke_id, &hub_asset).unwrap_or_default()
    }

    /// Returns the configured price aggregator address.
    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    /// Returns the configured minimum USD-denominated collateral floor (raw WAD)
    /// required to open a borrow position.
    fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    /// Returns whether `pool` is currently marked as an approved Blend pool.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        config::approvals::is_blend_pool_approved(&env, pool)
    }
}

#[contractimpl]
impl ControllerAdmin for Controller {
    /// Owner-only. Renews the controller instance TTL and sets the swap aggregator
    /// address to `addr`.
    #[only_owner]
    fn set_swap_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_swap_aggregator(&env, addr))
    }

    /// Owner-only. Renews the controller instance TTL and sets the price aggregator
    /// address to `addr`.
    #[only_owner]
    fn set_price_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_price_aggregator(&env, addr))
    }

    /// Owner-only. Renews the controller instance TTL and sets the revenue
    /// accumulator address to `addr`.
    #[only_owner]
    fn set_accumulator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_accumulator(&env, addr))
    }

    /// Owner-only. Renews the controller instance TTL and sets the maximum number of
    /// concurrent supply and borrow positions to `limits`. Panics if either limit is
    /// zero or exceeds the protocol maximum.
    #[only_owner]
    fn set_position_limits(env: Env, limits: PositionLimits) {
        renew_then!(env, config::limits::set_position_limits(&env, limits))
    }

    /// Owner-only. Renews the controller instance TTL and sets the minimum
    /// USD-denominated collateral floor (raw WAD) required to open a borrow position.
    /// Panics if `floor_wad` is negative.
    #[only_owner]
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        renew_then!(env, config::limits::set_min_borrow_collateral_usd(&env, floor_wad))
    }

    /// Owner-only. Renews the controller instance TTL and sets `manager`'s active
    /// status as a position manager.
    #[only_owner]
    fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        renew_then!(env, storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active }))
    }

    /// Owner-only. Renews the controller instance TTL and marks `pool` as an
    /// approved Blend pool.
    #[only_owner]
    fn approve_blend_pool(env: Env, pool: Address) {
        renew_then!(env, config::approvals::set_blend_pool_approval(&env, pool, true))
    }

    /// Owner-only. Renews the controller instance TTL and revokes `pool`'s Blend
    /// pool approval.
    #[only_owner]
    fn revoke_blend_pool(env: Env, pool: Address) {
        renew_then!(env, config::approvals::set_blend_pool_approval(&env, pool, false))
    }

    /// Owner-only. Renews the controller instance TTL, allocates a new hub with a
    /// fresh, incrementing ID, and stores it as active. Returns the new hub's ID.
    #[only_owner]
    fn create_hub(env: Env) -> u32 {
        renew_then!(env, config::hub::create_hub(&env))
    }

    /// Owner-only. Renews the controller instance TTL, allocates a new spoke ID, and
    /// stores a new spoke configuration with the default liquidation curve. Returns
    /// the assigned spoke ID.
    #[only_owner]
    fn add_spoke(env: Env) -> u32 {
        renew_then!(env, config::spoke::add_spoke(&env))
    }

    /// Owner-only. Renews the controller instance TTL and marks the spoke identified
    /// by `id` as deprecated. Panics if the spoke is already deprecated, or if no
    /// spoke exists for `id`.
    #[only_owner]
    fn remove_spoke(env: Env, id: u32) {
        renew_then!(env, config::spoke::remove_spoke(&env, id))
    }

    /// Owner-only. Renews the controller instance TTL, then validates and applies a
    /// new liquidation curve (target health factor, health factor for maximum bonus,
    /// and bonus factor in basis points) to the spoke identified by `id`. Panics if
    /// the curve parameters are invalid or if no spoke exists for `id`.
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

    /// Owner-only. Renews the controller instance TTL and lists a new asset in a
    /// spoke as described by `input`, validating risk bounds, liquidation fees, and
    /// caps. Panics if the spoke is deprecated or the asset is already listed.
    #[only_owner]
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::add_asset_to_spoke(&env, &input))
    }

    /// Owner-only. Renews the controller instance TTL and rewrites the full spoke
    /// asset listing described by `input`, including its `paused`/`frozen` flags
    /// (neither flag is ratcheted here). Panics if the asset is not currently listed
    /// in the spoke.
    #[only_owner]
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::edit_asset_in_spoke(&env, &input))
    }

    /// Owner-only. Renews the controller instance TTL and sets the `paused` and
    /// `frozen` flags on the spoke asset identified by `spoke_id` and `hub_asset`.
    /// Rejects any transition that would clear an already-set flag. Panics if the
    /// asset is not listed in the spoke.
    #[only_owner]
    fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        renew_then!(env, config::asset::set_spoke_asset_flags(&env, spoke_id, hub_asset, paused, frozen))
    }

    /// Owner-only. Renews the controller instance TTL and removes the listing for
    /// `hub_asset` from `spoke_id`. Panics if the asset is not listed in the spoke,
    /// or if it still has a nonzero supplied or borrowed balance.
    #[only_owner]
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        renew_then!(env, config::asset::remove_asset_from_spoke(&env, hub_asset, spoke_id))
    }

    /// Owner-only. Deploys the liquidity-pool contract under the controller's own
    /// address using a fixed salt, and stores its address. Panics if a pool is
    /// already deployed.
    #[only_owner]
    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address {
        markets::deploy_pool(&env, wasm_hash)
    }

    /// Owner-only. Registers a new market for `asset` under `hub_id` on the deployed
    /// liquidity pool. Panics if the hub is not active or if `params.asset_id` does
    /// not match `asset`.
    #[only_owner]
    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address {
        markets::create_liquidity_pool(&env, hub_id, asset, params)
    }

    /// Owner-only. Accrues the indexes for the market identified by `hub_asset` up
    /// to date, then updates its interest-rate model to `params`.
    #[only_owner]
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel) {
        markets::upgrade_liquidity_pool_params(&env, &hub_asset, &params);
    }

    /// Owner-only. Upgrades the deployed liquidity-pool contract to the WASM at
    /// `new_wasm_hash`.
    #[only_owner]
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        markets::upgrade_pool(&env, new_wasm_hash);
    }

    /// Owner-only. Socializes `account_id`'s bad debt provided its total debt
    /// exceeds its total collateral, bypassing the dust-capped threshold that
    /// `clean_bad_debt` requires. Requires that no flash loan is currently in
    /// progress.
    #[only_owner]
    fn force_socialize_bad_debt(env: Env, account_id: u64) {
        positions::liquidation::process_force_socialize_bad_debt(&env, account_id);
    }

    /// Owner-only. Renews the controller instance TTL and pauses the contract.
    #[only_owner]
    fn pause(env: Env) {
        governance::access::pause(&env);
    }

    /// Owner-only. Renews the controller instance TTL and unpauses the contract.
    #[only_owner]
    fn unpause(env: Env) {
        governance::access::unpause(&env);
    }

    /// Owner-only. Renews the controller instance TTL, pauses the contract if not
    /// already paused, and upgrades the contract to `new_wasm_hash`.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        governance::access::upgrade(&env, &new_wasm_hash);
    }

    /// Owner-only. Renews the controller instance TTL and stores `new_version` as
    /// the app version. Panics if `new_version` is not greater than the currently
    /// stored version.
    #[only_owner]
    fn migrate(env: Env, new_version: u32) {
        governance::access::migrate(&env, new_version);
    }

    /// Returns the stored app version, or the initial app version if none is
    /// stored.
    fn get_app_version(env: Env) -> u32 {
        governance::access::get_app_version(&env)
    }

    /// Owner-only. Renews the controller instance TTL and starts a two-step
    /// ownership transfer to `new_owner`, valid for acceptance until
    /// `live_until_ledger`.
    #[only_owner]
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        governance::access::transfer_ownership(&env, &new_owner, live_until_ledger);
    }

    /// Renews the controller instance TTL and completes a pending ownership
    /// transfer, making the caller the new owner.
    fn accept_ownership(env: Env) {
        governance::access::accept_ownership(&env);
    }
}
