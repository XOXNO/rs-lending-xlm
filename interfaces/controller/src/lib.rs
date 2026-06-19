#![no_std]
#![allow(clippy::too_many_arguments)]

pub mod admin;
pub mod types;

pub use admin::{ControllerAdmin, ControllerAdminClient};

use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};
use types::{
    AccountAttributes, AccountPositionRaw, AssetExtendedConfigView, DebtPositionRaw,
    EModeCategoryRaw, LiquidationEstimate, MarketConfig, MarketIndexRaw, MarketIndexView,
    PositionMode,
};

#[contractclient(name = "ControllerClient")]
/// Contract interface for lending accounts, markets, and views.
pub trait ControllerInterface {
    /// Supplies assets into an existing account or creates one when `account_id == 0`.
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        assets: Vec<(Address, i128)>,
    ) -> u64;

    /// Borrows assets after collateral, health-factor, cap, and oracle checks.
    fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(Address, i128)>);

    /// Withdraws collateral and rejects post-state LTV or health-factor
    /// violations. Tokens go to `to` when provided, else to `caller`; returns
    /// the actual amount paid per deduped asset (amount `0` closes the
    /// position and pays its floor-rounded value).
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(Address, i128)>,
        to: Option<Address>,
    ) -> Vec<(Address, i128)>;

    /// Repays debt for an account; account ownership is not required.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(Address, i128)>);

    /// Liquidates an underwater account and refunds payments above the close amount.
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    );

    /// Opens or adjusts a leveraged position through an opaque aggregator route.
    fn multiply(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        collateral_token: Address,
        debt_to_flash_loan: i128,
        debt_token: Address,
        mode: PositionMode,
        swap: Bytes,
        initial_payment: Option<(Address, i128)>,
        convert_swap: Option<Bytes>,
    ) -> u64;

    /// Swaps an existing debt asset into a new debt asset through the aggregator.
    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt_token: Address,
        amount: i128,
        new_debt_token: Address,
        swap: Bytes,
    );

    /// Swaps supplied collateral from one asset into another through the aggregator.
    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current_collateral: Address,
        amount: i128,
        new_collateral: Address,
        swap: Bytes,
    );

    /// Migrates a Blend V2 position (collateral, non-collateral supply, and
    /// debt) into the controller in one transaction at zero flash-loan fee.
    /// `account_id == 0` creates a fresh account. Collateral/supply are swept
    /// with withdraw-all semantics; each `(debt_asset, max)` bounds the zero-fee
    /// borrow used to clear that Blend debt. Returns the account id.
    fn migrate_from_blend(
        env: Env,
        caller: Address,
        account_id: u64,
        e_mode_category: u32,
        blend_pool: Address,
        collateral_assets: Vec<Address>,
        supply_assets: Vec<Address>,
        debt_caps: Vec<(Address, i128)>,
    ) -> u64;

    /// Uses collateral proceeds to repay debt through the aggregator.
    fn repay_debt_with_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        collateral_token: Address,
        collateral_amount: i128,
        debt_token: Address,
        swap: Bytes,
        close_position: bool,
    );

    /// Executes a flash loan of `asset` through the central pool and receiver contract.
    fn flash_loan(
        env: Env,
        caller: Address,
        asset: Address,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    /// Renews TTL for account storage keys after owner authorization.
    fn renew_account(env: Env, caller: Address, account_id: u64);

    /// Returns true when health factor is below one.
    fn can_be_liquidated(env: Env, account_id: u64) -> bool;

    /// Returns account health factor in WAD; debt-free accounts return `i128::MAX`.
    fn health_factor(env: Env, account_id: u64) -> i128;

    /// Returns total collateral value in USD WAD.
    fn total_collateral_in_usd(env: Env, account_id: u64) -> i128;

    /// Returns total borrow value in USD WAD.
    fn total_borrow_in_usd(env: Env, account_id: u64) -> i128;

    /// Returns current underlying collateral amount for one asset.
    fn collateral_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128;

    /// Returns current underlying debt amount for one asset.
    fn borrow_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128;

    /// Returns raw scaled supply and debt maps for an account.
    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<Address, AccountPositionRaw>,
        Map<Address, DebtPositionRaw>,
    );

    /// Returns whether `account_id` still has controller metadata on-chain.
    fn account_exists(env: Env, account_id: u64) -> bool;

    /// Returns whether `pool` is on the governance Blend-pool allow-list and may
    /// be used as a `migrate_from_blend` source.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool;

    /// Instance-level minimum LTV-weighted collateral USD WAD while debt exists.
    fn get_min_borrow_collateral_usd(env: Env) -> i128;

    /// Returns account mode and e-mode attributes.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    /// Returns market config for `asset`.
    fn get_market_config(env: Env, asset: Address) -> MarketConfig;

    /// Returns e-mode category config by id.
    fn get_e_mode_category(env: Env, category_id: u32) -> EModeCategoryRaw;

    /// Returns the central liquidity pool shared by every listed market.
    fn get_pool_address(env: Env) -> Address;

    /// Returns config and oracle data for each requested market.
    fn get_all_markets_detailed(env: Env, assets: Vec<Address>) -> Vec<AssetExtendedConfigView>;

    /// Returns indexes and price components for each requested market.
    fn get_all_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView>;

    /// Estimates liquidation seize, repay, refund, and bonus data.
    fn liquidation_estimations_detailed(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) -> LiquidationEstimate;

    /// Returns total collateral value available for liquidation, in USD WAD.
    fn liquidation_collateral_available(env: Env, account_id: u64) -> i128;

    /// Returns collateral value counted toward LTV, in USD WAD.
    fn ltv_collateral_in_usd(env: Env, account_id: u64) -> i128;

    /// Returns the largest `withdraw` amount of `asset` currently executable
    /// for `account_id` (position, pool cash, max-utilization cap, LTV/HF
    /// gates, dust floor); `0` while paused.
    fn max_withdraw(env: Env, account_id: u64, asset: Address) -> i128;

    /// Returns remaining supply-cap headroom for `asset` in asset units;
    /// `i128::MAX` when uncapped, `0` while paused or market not active.
    fn max_supply(env: Env, asset: Address) -> i128;

    /// Returns the largest `borrow` amount of `asset` currently executable for
    /// `account_id` (pool liquidity, max-utilization, borrow cap, LTV/HF
    /// gates); `0` while paused, on an inactive/non-borrowable
    /// market, or when the asset is structurally not borrowable for the account.
    fn max_borrow(env: Env, account_id: u64, asset: Address) -> i128;

    /// Returns supply/borrow indexes accrued to the current ledger timestamp;
    /// reads no oracle.
    fn get_market_index(env: Env, asset: Address) -> MarketIndexRaw;
}
