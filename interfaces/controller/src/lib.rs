#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only ABI mirror of the lending controller (production surface).
//!
//! `#[contractclient]` generates `ControllerClient`. Matches deployed
//! entrypoints by ABI name. Admin surface lives in `admin`.
//! Omits production entrypoints not wired for this client:
//! `update_indexes`, `claim_revenue`, `add_rewards`, `clean_bad_debt`,
//! `update_account_threshold`, `accept_ownership`, `get_app_version`,
//! `price_aggregator`.

pub mod admin;
pub use admin::{ControllerAdmin, ControllerAdminClient};
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate,
    MarketIndexRaw, MarketIndexView, PositionMode, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};

#[contractclient(name = "ControllerClient")]
/// Lending accounts, markets, and views.
pub trait ControllerInterface {
    /// Deposits `assets` as collateral and returns the account id. Caller auth.
    /// `account_id == 0` opens a new account on `spoke_id`; otherwise `spoke_id`
    /// is ignored. Owner/delegate for new slots; anyone may top up an existing leg.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `NotAuthorized` — a non-owner/non-delegate opens a new supply asset slot.
    /// * `HubNotActive` / `AssetNotInSpoke` / `SpokeAssetPaused` / `SpokeAssetFrozen` /
    ///   `NotCollateral` / `PositionLimitExceeded` — entry gates.
    /// * `SpokeSupplyCapReached` — deposit would exceed the spoke supply cap.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    /// Borrows `borrows` to `to` (default `caller`) on an existing account.
    /// Owner or active delegate. Re-checks LTV/HF on pool-returned indexes.
    ///
    /// # Errors
    /// * `NotAuthorized` — `caller` is neither owner nor active delegate.
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `HubNotActive` / `AssetNotInSpoke` / `SpokeAssetPaused` / `SpokeAssetFrozen` /
    ///   `AssetNotBorrowable` / `PositionLimitExceeded` — entry gates.
    /// * `SpokeBorrowCapReached` — borrow would exceed the spoke borrow cap.
    /// * `BorrowRoundsToZeroShares` — amount rounds to zero scaled debt (pool).
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — post-pool risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    );

    /// Withdraws collateral to `to` (default `caller`). Owner or active delegate.
    /// Amount `0` closes the leg. Returns gross pool `actual_amount` per asset.
    /// Re-checks LTV/HF when the account still has debt. Global pause does not block.
    ///
    /// # Errors
    /// * `NotAuthorized` — `caller` is neither owner nor active delegate.
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `SpokeAssetPaused` — spoke asset is paused (frozen may still withdraw).
    /// * `CollateralPositionNotFound` — no supply position for an asset.
    /// * `InsufficientLiquidity` — pool cannot cover the withdrawal.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — post-pool risk
    ///   gates on debt-bearing accounts.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn withdraw(
        env: Env,
        caller: Address,
        account_id: u64,
        withdrawals: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    ) -> Vec<(HubAssetKey, i128)>;

    /// Repays `payments` against `account_id`. Any caller may repay any account;
    /// payer auth covers the token transfer. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `SpokeAssetPaused` — spoke asset is paused (frozen may still repay).
    /// * `DebtPositionNotFound` — no debt position for an asset.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>);

    /// Liquidates an underwater account: liquidator pays selected debt and
    /// receives bonused collateral. Permissionless; liquidator auth; not the
    /// owner. Requires HF < 1. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `InvalidPayments` — empty debt payment list or empty post-normalization set.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `SelfLiquidationNotAllowed` — `liquidator` is the account owner.
    /// * `SpokeAssetPaused` — a repaid debt leg's listing is paused.
    /// * `HealthFactorTooHigh` — account HF is still at or above one.
    /// * `OracleNotConfigured` / `PoolNotInitialized` — fail-closed pricing path.
    ///
    /// # Events
    /// * topics — `["position", "liquidation"]`
    /// * topics — `["position", "batch_update"]`
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    );

    /// Opens or boosts a leveraged position via flash-loan debt → swap → supply.
    /// Owner or active delegate; `account_id == 0` creates on `spoke_id`.
    /// Returns the account id. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — flash-loan amount is not strictly positive.
    /// * `AssetsAreTheSame` / `InvalidPositionMode` — mode/asset preflight.
    /// * `NotCollateral` — destination collateral is not supply-enabled.
    /// * Entry/borrow/swap/deposit errors from the nested legs.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    /// * topics — `["strategy", "initial_payment"]` when `initial_payment` is set
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

    /// Refinances `amount` of `existing_debt` into `new_debt` via aggregator route.
    /// Owner or active delegate. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AssetsAreTheSame` — identical `(hub, asset)` pair.
    /// * `AmountMustBePositive` / `HubNotActive` — preflight.
    /// * `NotAuthorized` — caller is neither owner nor active delegate.
    /// * `DebtPositionNotFound` — no debt position for `existing_debt`.
    /// * Borrow/swap/repay errors from the nested legs.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn swap_debt(
        env: Env,
        caller: Address,
        account_id: u64,
        existing_debt: HubAssetKey,
        amount: i128,
        new_debt: HubAssetKey,
        swap: Bytes,
    );

    /// Swaps `amount` of supplied `current` into `new` via aggregator route.
    /// Owner or active delegate. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AssetsAreTheSame` — identical `(hub, asset)` pair.
    /// * `AmountMustBePositive` / `HubNotActive` — preflight.
    /// * `NotAuthorized` — caller is neither owner nor active delegate.
    /// * `NotCollateral` / `PositionLimitExceeded` — destination preflight.
    /// * `CollateralPositionNotFound` — no supply position for `current`.
    /// * Swap/deposit errors (`NoSwapOutput`, `RouterOverspend`, entry gates).
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    fn swap_collateral(
        env: Env,
        caller: Address,
        account_id: u64,
        current: HubAssetKey,
        amount: i128,
        new: HubAssetKey,
        swap: Bytes,
    );

    /// Migrates Blend V2 positions into the controller on `hub_id`.
    /// Caller auth; `account_id == 0` creates on `spoke_id`. Each debt cap
    /// bounds the zero-fee borrow that clears that Blend debt. Returns account id.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `HubNotActive` / `InvalidPayments` / `BlendPoolNotApproved` — preflight.
    /// * `AssetsAreTheSame` — duplicate debt asset in `debt_caps`.
    /// * `NotCollateral` / spoke pause-freeze — destination withdraw assets.
    /// * Borrow/repay/deposit errors from nested legs; Blend submit failures.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
    /// * topics — `["strategy", "blend_migration"]`
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

    /// Repays `debt` using `collateral_amount` of `collateral` (swap when distinct).
    /// Owner or active delegate. `close_position` fully exits remaining collateral
    /// only when debt is already zero. Finalizes with post-pool LTV/HF gates.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` / `HubNotActive` — preflight.
    /// * `NotAuthorized` — caller is neither owner nor active delegate.
    /// * `CollateralPositionNotFound` / `DebtPositionNotFound` — missing legs.
    /// * `CannotCloseWithRemainingDebt` — `close_position` while debt remains.
    /// * `InvalidPayments` — non-empty swap on same-asset net path.
    /// * Swap/withdraw/repay errors from the nested legs.
    /// * `InsufficientCollateral` / `MinBorrowCollateralNotMet` — finalize risk gates.
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "batch_update"]`
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

    /// Flash-loans `amount` of `asset` to `receiver` with opaque `data`.
    /// Caller auth. Pool enforces exact principal+fee repayment before return.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `AmountMustBePositive` — `amount` is not strictly positive.
    /// * `HubNotActive` — hub is inactive.
    /// * `InvalidFlashloanReceiver` — `receiver` is not a WASM contract.
    /// * Pool-side flash errors (`FlashloanNotEnabled`, `InvalidFlashloanRepay`, etc.).
    /// * The `#[when_not_paused]` guard reverts while the contract is paused.
    ///
    /// # Events
    /// * topics — `["position", "flash_loan"]`
    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    /// Extends the account's storage TTL. Account owner only.
    ///
    /// # Errors
    /// * `AccountNotInMarket` — missing account or `caller` is not the owner.
    fn renew_account(env: Env, caller: Address, account_id: u64);

    /// Registers `delegate` on `account_id` (effective only while `delegate` is
    /// also an active position manager). Account owner only.
    ///
    /// # Errors
    /// * `AccountNotInMarket` — missing account or `caller` is not the owner.
    /// * `RegistryCapReached` — delegate list is at capacity.
    fn add_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    /// Revokes `delegate` from `account_id`. Account owner only.
    ///
    /// # Errors
    /// * `AccountNotInMarket` — missing account or `caller` is not the owner.
    fn remove_delegate(env: Env, caller: Address, account_id: u64, delegate: Address);

    /// True when health factor is below one; a missing account is never
    /// liquidatable.
    ///
    /// # Errors
    /// * Pricing an indebted account reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    fn is_liquidatable(env: Env, account_id: u64) -> bool;

    /// Health factor in WAD; debt-free or missing accounts return `i128::MAX`.
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

    /// Whether `account_id` still has on-chain account metadata.
    fn account_exists(env: Env, account_id: u64) -> bool;

    /// Whether `pool` is on the Blend migration allowlist.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool;

    /// Returns the min-borrow-collateral floor (USD WAD).
    fn get_min_borrow_collateral_usd(env: Env) -> i128;

    /// Account mode and spoke attributes.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    /// Per-spoke risk listing for `hub_asset` on `spoke_id`.
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;

    /// Spoke config snapshot.
    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig;

    /// Scaled usage totals; zero when no row exists.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw;

    /// Central liquidity pool for all markets; reads instance storage only.
    fn get_pool_address(env: Env) -> Address;

    /// Pool indexes + soft oracle status for each requested hub-asset market.
    ///
    /// Oracle legs are diagnostic: `stale` / `deviation` set flags instead of
    /// trapping; `valid` is true only when the price is usable like the
    /// fail-closed solvency path. Prefer `get_pool_address` for the pool id.
    ///
    /// # Errors
    /// * `InvalidPayments` — `hub_assets` exceeds the view input bound.
    /// * `PoolNotInitialized` — a requested `(hub, asset)` market was never created.
    fn get_market_indexes_detailed(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexView>;

    /// Estimates the seize, repay, refund, and bonus data for liquidating the
    /// account with the supplied debt payments.
    ///
    /// # Errors
    /// * `InvalidPayments` — `debt_payments` exceeds the view input bound.
    /// * `AccountNotFound` — no account exists for `account_id`.
    /// * The liquidation engine reverts on oracle resolution or when the account
    ///   is not liquidatable; refer to the liquidation flow errors.
    fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate;

    /// Collateral available for liquidation (USD WAD).
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128;

    /// Collateral counted toward LTV (USD WAD).
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128;

    /// Largest executable `withdraw` amount.
    fn max_withdraw(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Supply-cap headroom for `account_id`; `i128::MAX` uncapped, `0` paused or inactive.
    fn max_supply(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Largest executable `borrow` amount of `hub_asset`; `0` while
    /// paused, on an inactive/non-borrowable market, or when the asset is
    /// structurally not borrowable for the account.
    fn max_borrow(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Accrued indexes; reads no oracle.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw;
}
