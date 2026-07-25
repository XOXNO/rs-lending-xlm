//! Owner and governance ABI of the lending controller (production surface).
//!
//! `#[contractclient]` generates `ControllerAdminClient` for governance
//! forwarding. The controller contract implements this trait, so the compiler
//! enforces that client and deployed entrypoints stay in step.

use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PositionLimits, SpokeAssetArgs,
};
use soroban_sdk::{contractclient, Address, BytesN, Env};

/// Owner-gated controller surface for governance forwarding.
#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    // --- wiring ---

    /// Sets the swap aggregator used by strategy swaps. Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdateSwapAggregatorEvent` — new aggregator address.
    fn set_swap_aggregator(env: Env, addr: Address);

    /// Sets the price aggregator (oracle authority). Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdatePriceAggregatorEvent` — new aggregator address.
    fn set_price_aggregator(env: Env, addr: Address);

    /// Sets the revenue accumulator for `claim_revenue`. Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdateAccumulatorEvent` — new accumulator address.
    fn set_accumulator(env: Env, addr: Address);

    /// Sets per-account max supply/borrow position counts. Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidPositionLimits` — a side is outside `1..=POSITION_LIMIT_MAX`.
    ///
    /// # Events
    /// * `UpdatePositionLimitsEvent` — new caps.
    fn set_position_limits(env: Env, limits: PositionLimits);

    /// Sets the min LTV-weighted collateral while debt is open (USD WAD ≥ 0).
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidBorrowParams` — `floor_wad` is negative.
    ///
    /// # Events
    /// * `UpdateMinBorrowCollateralEvent` — new floor.
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);

    /// Registers or removes a position manager (`false` deletes). Owner only
    /// (gov timelock).
    fn set_position_manager(env: Env, manager: Address, is_active: bool);

    /// Allows a Blend pool as a migration source. Owner only (gov timelock).
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` — `approved = true`.
    fn approve_blend_pool(env: Env, pool: Address);

    /// Revokes a Blend migration allowlist entry. Owner only (gov timelock).
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` — `approved = false`.
    fn revoke_blend_pool(env: Env, pool: Address);

    // --- hubs and spokes ---

    /// Creates an active hub; inert until markets list. Owner only (gov;
    /// GUARDIAN immediate path).
    ///
    /// # Events
    /// * `CreateHubEvent` — new hub id.
    fn create_hub(env: Env) -> u32;

    /// Creates a spoke with default liquidation curve. Owner only (gov;
    /// GUARDIAN immediate path).
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — new spoke snapshot.
    fn add_spoke(env: Env) -> u32;

    /// Deprecates a spoke (gates subsequent spoke reads). Owner only (gov timelock).
    ///
    /// # Errors
    /// * `SpokeNotFound` — unknown `id`.
    /// * `SpokeDeprecated` — already deprecated.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — deprecated snapshot.
    fn remove_spoke(env: Env, id: u32);

    /// Sets the spoke liquidation curve (target HF, knee, factor ≤ BPS).
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `SpokeNotFound` — unknown `id`.
    /// * `InvalidLiquidationCurve` — bounds violated.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — updated snapshot.
    fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    );

    /// Lists a hub-asset on a spoke; the pool market must already exist.
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidLiqThreshold` — risk bounds or liquidation fee out of range.
    /// * `InvalidBorrowParams` — negative cap or cap exceeds asset-decimal domain.
    /// * `SpokeNotFound` — unknown spoke.
    /// * `SpokeDeprecated` — spoke is deprecated.
    /// * `AssetAlreadyInSpoke` — listing already exists.
    /// * `PoolNotInitialized` — market never created.
    /// * `AssetDecimalsTooHigh` — market decimals exceed the RAY domain.
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` — resolved listing.
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs);

    /// Edits an existing listing (allowed on deprecated; caps may sit below
    /// usage). Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidLiqThreshold` — risk bounds or liquidation fee out of range.
    /// * `InvalidBorrowParams` — negative cap or cap exceeds asset-decimal domain.
    /// * `SpokeNotFound` — unknown spoke.
    /// * `AssetNotInSpoke` — listing missing.
    /// * `PoolNotInitialized` — market never created.
    /// * `AssetDecimalsTooHigh` — market decimals exceed the RAY domain.
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` — resolved listing.
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs);

    /// Tightens `paused`/`frozen` on a listing (clearing a flag reverts). Owner
    /// only (gov; GUARDIAN immediate path).
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    /// * `SpokeAssetFlagRelaxation` — a flag would clear (`true → false`).
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` — updated listing.
    fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    );

    /// Unlists a hub-asset when spoke usage is zero. Owner only (gov timelock).
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    /// * `SpokeAssetInUse` — scaled supply or borrow usage is non-zero.
    ///
    /// # Events
    /// * `RemoveSpokeAssetEvent` — removed listing key.
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32);

    // --- pool lifecycle ---

    /// Deploys the central liquidity pool once from `wasm_hash` (address from
    /// controller + salt). Owner only (gov timelock).
    ///
    /// # Errors
    /// * `PoolAlreadyDeployed` — pool already deployed.
    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address;

    /// Creates a `(hub_id, asset)` market on the deployed pool. Owner only
    /// (gov timelock).
    ///
    /// # Arguments
    /// * `asset` — must equal `params.asset_id`.
    ///
    /// # Errors
    /// * `HubNotActive` — hub missing or inactive.
    /// * `WrongToken` — `asset` ≠ `params.asset_id`.
    /// * `PoolNotInitialized` — central pool not deployed.
    /// * `AssetAlreadySupported` — market already exists.
    /// * `AssetDecimalsTooHigh` / `InvalidBorrowParams` / rate-model variants —
    ///   from pool `create_market` / `MarketParamsRaw::verify`.
    ///
    /// # Events
    /// * `CreateMarketEvent` — new market params snapshot.
    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address;

    /// Accrues indexes, then replaces the market interest-rate model. Owner
    /// only (gov timelock).
    ///
    /// # Errors
    /// * `PoolNotInitialized` — market or pool missing.
    /// * Rate-model variants from `InterestRateModel::verify`.
    ///
    /// # Events
    /// * `UpdateMarketParamsEvent` — new rate-model parameters.
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel);

    /// Upgrades the deployed central pool Wasm to `new_wasm_hash`. Owner only
    /// (gov timelock).
    ///
    /// # Errors
    /// * `PoolNotInitialized` — pool not deployed.
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);

    // --- emergency ---

    /// Socializes an underwater account's residual bad debt via the supply index
    /// with no token transfer. Owner only (gov timelock). Unlike the
    /// permissionless `clean_bad_debt`, this bypasses the dust-collateral
    /// threshold so debt on an account whose collateral cannot be seized
    /// (issuer-frozen or clawed-back) can still be retired. Requires the account
    /// to be genuinely underwater.
    ///
    /// # Errors
    /// * `FlashLoanOngoing` — a flash loan or strategy is mid-execution.
    /// * `DebtPositionNotFound` — the account carries no debt.
    /// * `CannotCleanBadDebt` — the account is not underwater (collateral >= debt).
    ///
    /// # Events
    /// * topics — `["debt", "bad_debt"]`
    fn force_socialize_bad_debt(env: Env, account_id: u64);

    // --- lifecycle and ownership ---

    /// Pauses the contract, blocking risk-increasing flows. Owner only (gov;
    /// GUARDIAN immediate path).
    fn pause(env: Env);

    /// Unpauses the contract, re-enabling risk-increasing flows. Owner only
    /// (gov timelock; no GUARDIAN unpause).
    fn unpause(env: Env);

    /// Pauses if needed, then upgrades controller Wasm to `new_wasm_hash`.
    /// Owner only (gov timelock).
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);

    /// Bumps the stored app version; strictly monotonic, no data rewrite.
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InternalError` — `new_version` is not greater than the current version.
    fn migrate(env: Env, new_version: u32);

    /// Returns the stored app version.
    fn get_app_version(env: Env) -> u32;

    /// Arms a two-step ownership transfer to `new_owner` until
    /// `live_until_ledger`; the pending owner must call `accept_ownership`.
    /// Owner only (gov timelock).
    ///
    /// # Arguments
    /// * `live_until_ledger` — offer expiry; `0` cancels the pending transfer.
    ///
    /// # Errors
    /// * `OwnerNotSet` — no current owner.
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);

    /// Completes a pending ownership transfer and syncs the access-control
    /// admin. Pending owner only (armed by `transfer_ownership`).
    ///
    /// # Errors
    /// * `OwnerNotSet` — no owner before or after the transfer.
    fn accept_ownership(env: Env);
}
