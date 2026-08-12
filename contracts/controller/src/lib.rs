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

#[cfg(test)]
#[path = "../tests/entrypoints.rs"]
mod entrypoint_tests;

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
    pub fn __constructor(env: Env, admin: Address) {
        governance::init(&env, &admin);
    }
}

#[contractimpl]
impl ControllerInterface for Controller {
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

    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)> {
        positions::process_withdraw(&env, &caller, account_id, &withdrawals, to)
    }

    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        positions::process_repay(&env, &caller, account_id, &payments);
    }

    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) {
        positions::liquidation::process_liquidation(&env, &liquidator, account_id, &debt_payments);
    }

    fn clean_bad_debt(env: Env, caller: Address, account_id: u64) {
        positions::liquidation::process_clean_bad_debt(&env, &caller, account_id);
    }

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

    #[when_not_paused]
    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>) {
        keepers::update_indexes(&env, caller, assets);
    }

    #[when_not_paused]
    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
        keepers::claim_revenue(&env, caller, assets)
    }

    #[when_not_paused]
    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>) {
        keepers::update_account_threshold(&env, caller, has_risks, account_ids);
    }

    fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128 {
        keepers::recapitalize(&env, payer, hub_asset, amount)
    }

    fn renew_account(env: Env, caller: Address, account_id: u64) {
        account::renew_account(&env, caller, account_id);
    }

    #[when_not_paused]
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::add_delegate(&env, caller, account_id, delegate);
    }

    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address) {
        account::remove_delegate(&env, caller, account_id, delegate);
    }

    fn is_liquidatable(env: Env, account_id: u64) -> bool {
        views::can_be_liquidated(&env, account_id)
    }

    fn get_health_factor(env: Env, account_id: u64) -> i128 {
        views::health_factor(&env, account_id)
    }

    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::total_collateral_in_usd(&env, account_id)
    }

    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        views::total_borrow_in_usd(&env, account_id)
    }

    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::collateral_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        views::borrow_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    ) {
        views::get_account_positions(&env, account_id)
    }

    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        views::get_account_attributes(&env, account_id)
    }

    fn account_exists(env: Env, account_id: u64) -> bool {
        views::account_exists(&env, account_id)
    }

    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate {
        views::liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        views::liquidation_collateral_available(&env, account_id)
    }

    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        views::ltv_collateral_in_usd(&env, account_id)
    }

    fn get_pool_address(env: Env) -> Address {
        views::get_pool_address(&env)
    }

    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw {
        let mut cache = context::Cache::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&hub_asset))
    }

    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView> {
        views::get_all_market_indexes_detailed(&env, &hub_assets)
    }

    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig {
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, SpokeError::AssetNotInSpoke))
    }

    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw {
        storage::get_spoke_usage(&env, spoke_id, &hub_asset).unwrap_or_default()
    }

    fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        config::registry::is_blend_pool_approved(&env, pool)
    }
}

#[contractimpl]
impl ControllerAdmin for Controller {
    #[only_owner]
    fn set_swap_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_swap_aggregator(&env, addr))
    }

    #[only_owner]
    fn set_price_aggregator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_price_aggregator(&env, addr))
    }

    #[only_owner]
    fn set_accumulator(env: Env, addr: Address) {
        renew_then!(env, config::registry::set_accumulator(&env, addr))
    }

    #[only_owner]
    fn set_position_limits(env: Env, limits: PositionLimits) {
        renew_then!(env, config::registry::set_position_limits(&env, limits))
    }

    #[only_owner]
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        renew_then!(env, config::registry::set_min_borrow_collateral_usd(&env, floor_wad))
    }

    #[only_owner]
    fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        renew_then!(env, storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active }))
    }

    #[only_owner]
    fn approve_blend_pool(env: Env, pool: Address) {
        renew_then!(env, config::registry::set_blend_pool_approval(&env, pool, true))
    }

    #[only_owner]
    fn revoke_blend_pool(env: Env, pool: Address) {
        renew_then!(env, config::registry::set_blend_pool_approval(&env, pool, false))
    }

    #[only_owner]
    fn create_hub(env: Env) -> u32 {
        renew_then!(env, config::spoke::create_hub(&env))
    }

    #[only_owner]
    fn add_spoke(env: Env) -> u32 {
        renew_then!(env, config::spoke::add_spoke(&env))
    }

    #[only_owner]
    fn remove_spoke(env: Env, id: u32) {
        renew_then!(env, config::spoke::remove_spoke(&env, id))
    }

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

    #[only_owner]
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::add_asset_to_spoke(&env, &input))
    }

    #[only_owner]
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        renew_then!(env, config::asset::edit_asset_in_spoke(&env, &input))
    }

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

    #[only_owner]
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        renew_then!(env, config::asset::remove_asset_from_spoke(&env, hub_asset, spoke_id))
    }

    #[only_owner]
    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address {
        renew_then!(env, markets::deploy_pool(&env, wasm_hash))
    }

    #[only_owner]
    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address {
        renew_then!(env, markets::create_liquidity_pool(&env, hub_id, asset, params))
    }

    #[only_owner]
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel) {
        renew_then!(env, markets::upgrade_liquidity_pool_params(&env, &hub_asset, &params))
    }

    #[only_owner]
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, markets::upgrade_pool(&env, new_wasm_hash))
    }

    #[only_owner]
    fn force_socialize_bad_debt(env: Env, account_id: u64) {
        renew_then!(env, positions::liquidation::process_force_socialize_bad_debt(&env, account_id))
    }

    #[only_owner]
    fn pause(env: Env) {
        renew_then!(env, governance::pause(&env))
    }

    #[only_owner]
    fn unpause(env: Env) {
        renew_then!(env, governance::unpause(&env))
    }

    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_then!(env, governance::upgrade(&env, &new_wasm_hash))
    }

    #[only_owner]
    fn migrate(env: Env, new_version: u32) {
        renew_then!(env, governance::migrate(&env, new_version))
    }

    fn get_app_version(env: Env) -> u32 {
        governance::get_app_version(&env)
    }

    #[only_owner]
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        renew_then!(env, governance::transfer_ownership(&env, &new_owner, live_until_ledger))
    }

    fn accept_ownership(env: Env) {
        governance::accept_ownership(&env);
    }
}
