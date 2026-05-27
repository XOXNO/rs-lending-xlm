#![no_std]
#![allow(clippy::too_many_arguments)]

use common::types::{
    AccountAttributes, AccountPositionRaw, AggregatorSwap, AssetExtendedConfigView,
    DebtPositionRaw, EModeCategoryRaw, LiquidationEstimate, MarketConfig, MarketIndexView,
    PositionMode,
};
use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};

#[contractclient(name = "ControllerClient")]
/// Primary user-facing contract interface for the lending protocol.
///
/// All position mutations (supply/borrow/withdraw/repay/liquidate) and the
/// four strategies go through this surface. The controller owns all account
/// state, risk parameters, oracle policy, and authorization; pools only
/// custody tokens and interest math.
///
/// See `architecture/` for invariants and the per-decision ADRs.
pub trait ControllerInterface {
    // --- Position operations ------------------------------------------------

    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<(Address, i128)>,
    ) -> u64;

    fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>);

    fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<(Address, i128)>);

    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(Address, i128)>);

    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    );

    // --- Strategies ---------------------------------------------------------

    fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        collateral_token: Address,
        debt_to_flash_loan: i128,
        debt_token: Address,
        mode: PositionMode,
        swap: AggregatorSwap,
        initial_payment: Option<(Address, i128)>,
        convert_swap: Option<AggregatorSwap>,
    ) -> u64;

    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt_token: Address,
        amount: i128,
        new_debt_token: Address,
        swap: AggregatorSwap,
    );

    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        swap: AggregatorSwap,
    );

    fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral_token: Address,
        collateral_amount: i128,
        debt_token: Address,
        swap: AggregatorSwap,
        close_position: bool,
    );

    // --- Flash loan and account TTL ----------------------------------------

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: Address,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    fn renew_account(env: Env, caller: Address, account_id: u64);

    // --- Views --------------------------------------------------------------

    fn can_be_liquidated(env: Env, account_id: u64) -> bool;

    fn health_factor(env: Env, account_id: u64) -> i128;

    fn total_collateral_in_usd(env: Env, account_id: u64) -> i128;

    fn total_borrow_in_usd(env: Env, account_id: u64) -> i128;

    fn collateral_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128;

    fn borrow_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128;

    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<Address, AccountPositionRaw>,
        Map<Address, DebtPositionRaw>,
    );

    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    fn get_market_config(env: Env, asset: Address) -> MarketConfig;

    fn get_e_mode_category(env: Env, category_id: u32) -> EModeCategoryRaw;

    fn get_isolated_debt(env: Env, asset: Address) -> i128;

    fn get_all_markets_detailed(env: Env, assets: Vec<Address>) -> Vec<AssetExtendedConfigView>;

    fn get_all_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView>;

    fn liquidation_estimations_detailed(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) -> LiquidationEstimate;

    fn liquidation_collateral_available(env: Env, account_id: u64) -> i128;

    fn ltv_collateral_in_usd(env: Env, account_id: u64) -> i128;
}
