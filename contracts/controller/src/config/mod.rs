//! Owner-gated protocol configuration entrypoints.

mod approvals;
mod asset;
mod hub;
mod limits;
pub(crate) mod oracle;
mod position_manager;
mod spoke;

#[cfg(feature = "certora")]
pub(crate) use asset::{add_asset_to_spoke, edit_asset_in_spoke};
pub(crate) use hub::require_hub_active;
#[cfg(feature = "certora")]
pub(crate) use spoke::remove_spoke;

#[cfg(test)]
use common::types::HubConfig;
use common::types::{
    HubAssetKey, MarketOracleConfig, OraclePriceFluctuation, PositionLimits, SpokeAssetArgs,
};
use soroban_sdk::{contractimpl, Address, BytesN, Env};
use stellar_macros::only_owner;

use crate::{storage, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Sets the swap-aggregator contract used by every controller strategy.
    ///
    /// # Events
    /// * `UpdateAggregatorEvent` - the new aggregator address.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_aggregator(&env, addr);
    }

    /// Sets the revenue-accumulator contract that claimed protocol revenue is
    /// forwarded to.
    ///
    /// # Events
    /// * `UpdateAccumulatorEvent` - the new accumulator address.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_accumulator(&env, addr);
    }

    /// Sets the Wasm hash used to deploy the central liquidity pool.
    ///
    /// # Events
    /// * `UpdatePoolTemplateEvent` - the new template hash.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        approvals::set_liquidity_pool_template(&env, hash);
    }

    /// Sets the instance-wide caps on the number of supply and borrow positions
    /// an account may hold.
    ///
    /// # Events
    /// * `UpdatePositionLimitsEvent` - the new supply/borrow position caps.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        limits::set_position_limits(&env, limits);
    }

    /// Sets the instance minimum LTV-weighted collateral (USD WAD) required
    /// while an account carries debt.
    ///
    /// # Arguments
    /// * `floor_wad` - USD WAD floor; must be non-negative.
    ///
    /// # Errors
    /// * `InvalidBorrowParams` - `floor_wad` is negative.
    ///
    /// # Events
    /// * `UpdateMinBorrowCollateralEvent` - the new floor.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        limits::set_min_borrow_collateral_usd(&env, floor_wad);
    }

    /// Returns the instance minimum LTV-weighted collateral floor (USD WAD).
    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        limits::get_min_borrow_collateral_usd(&env)
    }

    /// Registers a new active hub and returns its assigned id.
    ///
    /// # Events
    /// * `CreateHubEvent` - the new hub id.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn create_hub(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        hub::create_hub(&env)
    }

    /// Registers a new spoke (stamped with the default liquidation curve) and
    /// returns its assigned id.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` - the new spoke snapshot.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn add_spoke(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        spoke::add_spoke(&env)
    }

    /// Deprecates a spoke, which gates all subsequent reads of it.
    ///
    /// # Errors
    /// * `SpokeNotFound` - no spoke exists for `id`.
    /// * `SpokeDeprecated` - the spoke is already deprecated.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` - the deprecated spoke snapshot.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn remove_spoke(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        spoke::remove_spoke(&env, id);
    }

    /// Overrides a spoke's liquidation curve: the health factor a liquidated
    /// position is restored to (`target_hf_wad`), the health factor at/below
    /// which the max bonus applies (`hf_for_max_bonus_wad`), and the factor
    /// scaling the bonus increment between them
    /// (`liquidation_bonus_factor_bps`). Replaces the defaults stamped at
    /// spoke creation.
    ///
    /// # Errors
    /// * `SpokeNotFound` - no spoke exists for `id`.
    /// * `InvalidLiquidationCurve` - `target_hf_wad <= 1.0 WAD`,
    ///   `hf_for_max_bonus_wad` is outside `(0, target_hf_wad)`, or
    ///   `liquidation_bonus_factor_bps` exceeds `BPS` (100%).
    ///
    /// # Events
    /// * `UpdateSpokeEvent` - the updated spoke snapshot.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    ) {
        storage::renew_controller_instance(&env);
        spoke::set_spoke_liquidation_curve(
            &env,
            id,
            target_hf_wad,
            hf_for_max_bonus_wad,
            liquidation_bonus_factor_bps,
        );
    }

    /// Lists a hub-asset on a spoke with its risk params, caps, and optional
    /// oracle override. The market must already exist in the pool.
    ///
    /// # Errors
    /// * `InvalidLiqThreshold` - risk bounds (LTV / threshold / bonus) or the
    ///   liquidation fee are out of range.
    /// * `InvalidBorrowParams` - a supply or borrow cap is negative or exceeds
    ///   the asset-decimal rescale domain.
    /// * `SpokeNotFound` - no spoke exists for the requested id.
    /// * `SpokeDeprecated` - the spoke is deprecated.
    /// * `AssetAlreadyInSpoke` - the asset is already listed on the spoke.
    /// * `PoolNotInitialized` - the `(hub, asset)` market was never created.
    /// * `AssetDecimalsTooHigh` - the market's asset decimals exceed the RAY domain.
    /// * Oracle override (when `Some`): `InvalidSanityBounds`, `InvalidOracleBase`,
    ///   or `InvalidAsset` (override decimals disagree with the market).
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` - the resolved spoke-asset config.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::add_asset_to_spoke(&env, &input);
    }

    /// Updates an existing spoke-asset listing's risk params, caps, and optional
    /// oracle override; new caps may not drop below current spoke usage.
    ///
    /// # Errors
    /// * `InvalidLiqThreshold` - risk bounds (LTV / threshold / bonus) or the
    ///   liquidation fee are out of range.
    /// * `InvalidBorrowParams` - a supply or borrow cap is negative or exceeds
    ///   the asset-decimal rescale domain.
    /// * `SpokeNotFound` - no spoke exists for the requested id.
    /// * `SpokeDeprecated` - the spoke is deprecated.
    /// * `AssetNotInSpoke` - the asset is not currently listed on the spoke.
    /// * `PoolNotInitialized` - the `(hub, asset)` market was never created.
    /// * `AssetDecimalsTooHigh` - the market's asset decimals exceed the RAY domain.
    /// * `SpokeCapBelowUsage` - a new cap is below current scaled spoke usage.
    /// * Oracle override (when `Some`): `InvalidSanityBounds`, `InvalidOracleBase`,
    ///   or `InvalidAsset` (override decimals disagree with the market).
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` - the resolved spoke-asset config.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::edit_asset_in_spoke(&env, &input);
    }

    /// Unlists a hub-asset from a spoke.
    ///
    /// # Errors
    /// * `AssetNotInSpoke` - the asset is not listed on the spoke.
    ///
    /// # Events
    /// * `RemoveSpokeAssetEvent` - the removed asset and spoke.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        asset::remove_asset_from_spoke(&env, hub_asset, spoke_id);
    }

    /// Adds `token` to the market-creation approval allowlist.
    ///
    /// # Errors
    /// * `RegistryCapReached` - the approved-token registry is full.
    ///
    /// # Events
    /// * `ApproveTokenEvent` - the token hash and approved flag.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        approvals::set_token_approval(&env, token, true);
    }

    /// Removes `token` from the market-creation approval allowlist.
    ///
    /// # Events
    /// * `ApproveTokenEvent` - the token hash and cleared flag.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn revoke_token(env: Env, token: Address) {
        approvals::set_token_approval(&env, token, false);
    }

    /// Returns whether `pool` is on the Blend-pool migration allowlist.
    pub fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        approvals::is_blend_pool_approved(&env, pool)
    }

    /// Adds `pool` to the Blend-pool allowlist that `migrate_from_blend` accepts
    /// as a migration source.
    ///
    /// # Errors
    /// * `RegistryCapReached` - the approved Blend-pool registry is full.
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` - the pool and approved flag.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn approve_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, true);
    }

    /// Removes `pool` from the Blend-pool migration allowlist.
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` - the pool and cleared flag.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, false);
    }

    /// Configures the token-rooted oracle for an already-created market. The
    /// oracle is keyed by the bare asset (hub-independent).
    ///
    /// # Errors
    /// * `PoolNotInitialized` - the `(hub, asset)` market was never created.
    /// * `InvalidSanityBounds` - the sanity price bounds are inconsistent.
    /// * `InvalidOracleBase` - a quoted source self-quotes, its quote asset has
    ///   no oracle, or that quote is not itself USD-based.
    ///
    /// # Events
    /// * `UpdateAssetOracleEvent` - the resolved oracle provider snapshot.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_market_oracle_config(env: Env, hub_asset: HubAssetKey, config: MarketOracleConfig) {
        storage::renew_controller_instance(&env);
        oracle::set_market_oracle_config(&env, hub_asset, config);
    }

    /// Updates the price-fluctuation tolerance on an active asset oracle.
    ///
    /// # Errors
    /// * `PairNotActive` - the asset has no configured oracle.
    ///
    /// # Events
    /// * `UpdateAssetOracleEvent` - the updated oracle provider snapshot.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation) {
        storage::renew_controller_instance(&env);
        oracle::set_oracle_tolerance(&env, asset, tolerance);
    }

    /// Registers or deactivates a position manager. Absence of an entry is the
    /// inactive signal, so deactivation removes it.
    ///
    /// # Arguments
    /// * `is_active` - `true` registers/activates `manager`; `false` removes it.
    ///
    /// # Errors
    /// * `RegistryCapReached` - activating would exceed the position-manager cap.
    ///
    /// # Security Warning
    /// * Owner-only via `#[only_owner]`; the owner is the governance timelock,
    ///   so this executes only after the configured delay.
    #[only_owner]
    pub fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        storage::renew_controller_instance(&env);
        position_manager::set_position_manager(&env, manager, is_active);
    }
}

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
