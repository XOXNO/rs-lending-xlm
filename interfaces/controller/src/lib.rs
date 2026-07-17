#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only controller ABI (`ControllerClient`). Matches deploy entrypoints by name.
//! Admin surface lives in `admin`.

pub mod admin;
pub use admin::{ControllerAdmin, ControllerAdminClient};
use common::types::{
    AccountAttributes, AccountPositionRaw, AssetExtendedConfigView, DebtPositionRaw, HubAssetKey,
    LiquidationEstimate, MarketIndexRaw, MarketIndexView, PositionMode, SpokeAssetConfig,
    SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};

#[contractclient(name = "ControllerClient")]
/// Lending accounts, markets, and views.
pub trait ControllerInterface {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    /// Collateral, HF, cap, and oracle checks. Tokens go to `to` when set, else
    /// `caller`; debt is always recorded on `account_id`.
    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    );

    /// `caller`. Returns actual amount paid per deduped asset (`0` closes and
    /// pays the floor-rounded value).
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)>;

    /// Anyone may repay; ownership not required.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>);

    /// Liquidates an underwater account; refunds payments above the close amount.
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    );

    /// Leveraged position via opaque aggregator route.
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

    /// Refinances debt via aggregator; hubs may differ for `existing_debt` / `new_debt`.
    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    );

    /// Swaps supplied collateral via aggregator.
    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    );

    /// Blend V2 → controller in one tx at zero flash-loan fee. `account_id == 0`
    /// creates a fresh account. Opened positions land on `hub_id`. Collateral/
    /// supply use withdraw-all; each `(debt_asset, max)` bounds the zero-fee
    /// borrow that clears that Blend debt. Returns the account id.
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

    /// Collateral proceeds → debt via aggregator.
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

    /// Flash loan of `asset` through the central pool and receiver.
    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    /// Renews TTL for account storage keys after owner auth.
    fn renew_account(env: Env, caller: Address, account_id: u64);

    /// Owner-only. Effective only while `delegate` is a registered, active
    /// position manager. Idempotent.
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    /// Owner-only revoke of `delegate` on `account_id`.
    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    /// True when health factor is below one.
    fn is_liquidatable(env: Env, account_id: u64) -> bool;

    /// Health factor in WAD; debt-free accounts return `i128::MAX`.
    fn get_health_factor(env: Env, account_id: u64) -> i128;

    /// Total collateral value (USD WAD).
    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128;

    /// Total borrow value (USD WAD).
    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128;

    /// Current underlying collateral for one hub-asset.
    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Current underlying debt for one hub-asset.
    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Raw scaled supply and debt maps.
    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    );

    /// Whether `account_id` still has controller metadata on-chain.
    fn account_exists(env: Env, account_id: u64) -> bool;

    /// Whether `pool` is on the Blend-pool allow-list for `migrate_from_blend`.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool;

    /// Instance-level minimum LTV-weighted collateral USD WAD while debt exists.
    fn get_min_borrow_collateral_usd(env: Env) -> i128;

    /// Account mode and spoke attributes.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    /// Per-spoke risk listing for `hub_asset` on `spoke_id` (id `>= 1`).
    /// Panics `AssetNotInSpoke` when not listed.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;

    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig;

    /// Scaled usage totals; zero when no row exists.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw;

    /// Central liquidity pool shared by every listed market.
    fn get_pool_address(env: Env) -> Address;

    /// Config and oracle data for each requested hub-asset market.
    fn get_markets_detailed(env: Env, hub_assets: Vec<HubAssetKey>)
        -> Vec<AssetExtendedConfigView>;

    /// Indexes and price components for each requested hub-asset market.
    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>;

    /// Seize, repay, refund, and bonus estimate.
    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate;

    /// Collateral available for liquidation (USD WAD).
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128;

    /// Collateral counted toward LTV (USD WAD).
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128;

    /// Largest executable `withdraw` of `hub_asset` (position, pool cash,
    /// max-util, LTV/HF, dust); `0` while paused.
    fn max_withdraw(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Supply-cap headroom in asset units; `i128::MAX` uncapped, `0` paused or inactive.
    fn max_supply(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Largest executable `borrow` of `hub_asset` (liquidity, max-util, cap,
    /// LTV/HF); `0` while paused, inactive/non-borrowable, or structurally blocked.
    fn max_borrow(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Supply/borrow indexes accrued to current ledger time; no oracle read.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw;
}
