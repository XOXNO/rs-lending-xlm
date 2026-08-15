#![no_std]
#![allow(clippy::too_many_arguments)]

pub mod admin;
pub use admin::{ControllerAdmin, ControllerAdminClient};
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate,
    MarketIndexRaw, MarketIndexView, PositionMode, SeizeMode, SpokeAssetConfig, SpokeConfig,
    SpokeUsageRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};

#[contractclient(name = "ControllerClient")]
pub trait ControllerInterface {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    );

    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)>;

    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>);

    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> u64;

    fn clean_bad_debt(env: Env, caller: Address, account_id: u64);

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

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
    ) -> u64;

    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    );

    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    );

    fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral: HubAssetKey,
        collateral_amount: i128,
        debt: HubAssetKey,
        swap: Bytes,
        close_position: bool,
    );

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
    ) -> u64;

    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>);

    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128>;

    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>);

    fn recapitalize(env: Env, payer: Address, hub_asset: HubAssetKey, amount: i128) -> i128;

    fn renew_account(env: Env, caller: Address, account_id: u64);

    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    fn is_liquidatable(env: Env, account_id: u64) -> bool;

    fn get_health_factor(env: Env, account_id: u64) -> i128;

    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128;

    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128;

    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    );

    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    fn account_exists(env: Env, account_id: u64) -> bool;

    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> LiquidationEstimate;

    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128;

    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128;

    fn get_pool_address(env: Env) -> Address;

    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw;

    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>;

    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig;

    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;

    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw;

    fn price_aggregator(env: Env) -> Address;

    fn get_min_borrow_collateral_usd(env: Env) -> i128;

    fn is_blend_pool_approved(env: Env, pool: Address) -> bool;
}
