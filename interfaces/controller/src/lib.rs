#![no_std]
#![allow(clippy::too_many_arguments)]

//! Non-admin ABI of the lending controller (production surface).
//!
//! `#[contractclient]` generates `ControllerClient`. The controller contract
//! implements this trait, so the compiler enforces that client and deployed
//! entrypoints stay in step. Owner/governance surface lives in `admin`.

pub mod admin;
pub use admin::{ControllerAdmin, ControllerAdminClient};
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate,
    MarketIndexRaw, MarketIndexView, PositionMode, SpokeAssetConfig, SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::{contractclient, Address, Bytes, Env, Map, Vec};

/// Lending accounts, markets, and views.
#[contractclient(name = "ControllerClient")]
pub trait ControllerInterface {
    // --- money paths ---

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

    // --- liquidation ---

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

    /// Socializes residual bad debt into the pool (no liquidator proceeds).
    /// Permissionless; caller auth for accountability. Global pause does not block.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `DebtPositionNotFound` — the account carries no debt.
    /// * `CannotCleanBadDebt` — not eligible socializable residual.
    ///
    /// # Events
    /// * topics — `["debt", "bad_debt"]`
    fn clean_bad_debt(env: Env, caller: Address, account_id: u64);

    // --- strategies ---

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

    // --- keepers: permissionless upkeep ---

    /// Accrues interest for each listed hub-asset market. Permissionless
    /// (caller auth); blocked while paused.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `PoolNotInitialized` — a listed market has not been created.
    fn update_indexes(env: Env, caller: Address, assets: Vec<HubAssetKey>);

    /// Reconciles a market's tracked cash to the live pool balance after an
    /// issuer clawback, socializing the shortfall through the supply index.
    /// Permissionless (caller auth); blocked while paused. Safe to open: the pool
    /// only ever writes cash down to the real balance, so no caller can profit or
    /// impose a loss beyond the clawback that already happened.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `PoolNotInitialized` — the market has not been created.
    fn reconcile_pool_reserves(env: Env, caller: Address, hub_asset: HubAssetKey);

    /// Claims protocol revenue per market and forwards it to the accumulator.
    /// Permissionless (caller auth); blocked while paused.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `NoAccumulator` — no revenue accumulator configured.
    /// * `PoolNotInitialized` — a listed market has not been created.
    fn claim_revenue(env: Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128>;

    /// Transfers external supply rewards into markets, raising each supply
    /// index. Permissionless (caller auth + token transfer); blocked while paused.
    ///
    /// # Arguments
    /// * `rewards` — `(hub-asset, amount)` legs; amounts must be positive.
    ///   Legs targeting the same market are summed into one distribution.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `InvalidPayments` — the `rewards` batch is empty.
    /// * `AmountMustBePositive` — a leg amount is not strictly positive.
    /// * `PoolNotInitialized` — a target market has not been created.
    /// * `NoSuppliersToReward` — a target market has no suppliers.
    /// * `SupplyIndexRewardCeiling` — a reward would lift a market's supply index
    ///   past its reward-growth ceiling.
    fn add_rewards(env: Env, caller: Address, rewards: Vec<(HubAssetKey, i128)>);

    /// Propagates spoke risk params onto supply positions. Permissionless
    /// (caller auth). LTV always re-stamps. When `has_risks`, each leg's
    /// liquidation tuple (threshold, bonus, fees) re-stamps too and the whole
    /// call must leave the account at or above the min HF.
    ///
    /// That gate bounds the threshold, which feeds the health factor. Bonus and
    /// fees do not enter the HF computation, so the gate does not bound them —
    /// it only reverts the entire stamp for an account that is already near
    /// liquidation. Every field is copied from the spoke listing, so a caller
    /// can only write governance's configured values, never arbitrary ones; a
    /// healthy account's bonus may be raised to the current config value.
    /// Delisted spoke members keep stamped params and are skipped. Blocked
    /// while paused.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `HealthFactorTooLow` — re-stamping the liquidation tuple would leave
    ///   the account below the min HF.
    ///
    /// # Events
    /// * Position-batch event per updated account.
    fn update_account_threshold(env: Env, caller: Address, has_risks: bool, account_ids: Vec<u64>);

    // --- account ops ---

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

    // --- views: account health and positions ---

    /// True when the account's health factor is below one (WAD). A debt-free or
    /// missing account reads `i128::MAX` and is therefore never liquidatable.
    ///
    /// # Errors
    /// * Pricing an indebted account reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a held market has no pool state.
    fn is_liquidatable(env: Env, account_id: u64) -> bool;

    /// Returns the health factor in WAD (raw 1e18): floor-valued,
    /// liquidation-threshold-weighted collateral over ceil-valued debt.
    ///
    /// A debt-free or missing account returns `i128::MAX` and reads no oracle;
    /// a ratio too large to represent saturates at `i128::MAX` rather than
    /// reverting.
    ///
    /// # Errors
    /// * Pricing an indebted account reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a held market has no pool state.
    fn get_health_factor(env: Env, account_id: u64) -> i128;

    /// Returns the unweighted USD value of every supply leg in WAD (raw 1e18),
    /// half-up at each step. No LTV or liquidation threshold is applied, so this
    /// is a reporting figure, not a gate input. A missing account or one with no
    /// supply legs returns `0`.
    ///
    /// # Errors
    /// * Pricing reads oracles and can revert (e.g. `OracleNotConfigured`,
    ///   `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a supplied market has no pool state.
    fn get_total_collateral_usd(env: Env, account_id: u64) -> i128;

    /// Returns the USD value of every debt leg in WAD (raw 1e18), half-up at
    /// each step. Solvency gates value debt with ceil rounding instead, so this
    /// is a reporting figure, not the amount owed. A missing account or one with
    /// no debt legs returns `0`.
    ///
    /// # Errors
    /// * Pricing reads oracles and can revert (e.g. `OracleNotConfigured`,
    ///   `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a borrowed market has no pool state.
    fn get_total_borrow_usd(env: Env, account_id: u64) -> i128;

    /// Returns the underlying collateral for one hub-asset in that market's
    /// asset decimals, valued at the live supply index with half-up rounding: a
    /// reporting figure, not a payable amount. Reads no oracle. An account with
    /// no supply leg for `hub_asset` returns `0`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — the market has no pool state.
    fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Returns the underlying debt for one hub-asset in that market's asset
    /// decimals, valued at the live borrow index with half-up rounding: a
    /// reporting figure, not the exact payoff amount. Reads no oracle. An
    /// account with no debt leg for `hub_asset` returns `0`.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — the market has no pool state.
    fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128;

    /// Returns the stored supply and debt maps keyed by hub-asset. Both sides
    /// carry RAY-scaled shares, not underlying: multiply by the market's supply
    /// or borrow index to get asset units. Supply entries also carry the risk
    /// params in BPS stamped at the leg's last touch. A missing account returns
    /// two empty maps. Reads storage only — no oracle and no pool call.
    fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    );

    /// Returns the account's spoke id and position mode. Reads storage only.
    ///
    /// # Errors
    /// * `AccountNotInMarket` — no account metadata for `account_id`.
    fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes;

    /// Whether `account_id` still has on-chain account metadata.
    fn account_exists(env: Env, account_id: u64) -> bool;

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

    /// Returns liquidation-threshold-weighted collateral in USD WAD (raw 1e18),
    /// the health-factor numerator. Position value and threshold weighting both
    /// floor, so the figure never rounds in the account's favor. A missing
    /// account returns `0`.
    ///
    /// # Errors
    /// * Pricing reads oracles and can revert (e.g. `OracleNotConfigured`,
    ///   `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a held market has no pool state.
    fn get_liquidation_collateral(env: Env, account_id: u64) -> i128;

    /// Returns LTV-weighted collateral in USD WAD (raw 1e18), the borrow
    /// capacity numerator. Floors at every step so capacity cannot round upward.
    /// Each still-listed leg's LTV is re-stamped from the spoke listing in
    /// memory first, so the figure tracks current risk params without writing
    /// storage. A missing account returns `0`.
    ///
    /// # Errors
    /// * Pricing reads oracles and can revert (e.g. `OracleNotConfigured`,
    ///   `PriceFeedStale`, `UnsafePriceNotAllowed`).
    /// * `PoolNotInitialized` — a supplied market has no pool state.
    fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128;

    // --- views: markets and registry ---

    /// Central liquidity pool for all markets; reads instance storage only.
    fn get_pool_address(env: Env) -> Address;

    /// Accrued indexes; reads no oracle.
    fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw;

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

    /// Returns the spoke record: the deprecation flag plus the liquidation
    /// curve, whose target and max-bonus health factors are WAD and whose bonus
    /// factor is BPS. Deprecated spokes are returned like any other.
    ///
    /// # Errors
    /// * `SpokeNotFound` — no spoke record for `spoke_id`.
    fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig;

    /// Returns the per-spoke listing for `hub_asset`: collateral and borrow
    /// flags, pause and freeze flags, risk params in BPS, and supply/borrow caps
    /// in asset-native units.
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;

    /// Returns the spoke's aggregate RAY-scaled supply and borrow shares for
    /// `hub_asset`, the basis cap accounting works in, not underlying amounts.
    /// A spoke-asset with no usage row returns a zeroed record.
    fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw;

    // --- views: configuration ---

    /// Returns the wired price aggregator.
    ///
    /// # Errors
    /// * `AggregatorNotSet` — no aggregator configured.
    fn price_aggregator(env: Env) -> Address;

    /// Returns the min-borrow-collateral floor (USD WAD).
    fn get_min_borrow_collateral_usd(env: Env) -> i128;

    /// Whether `pool` is on the Blend migration allowlist.
    fn is_blend_pool_approved(env: Env, pool: Address) -> bool;
}
