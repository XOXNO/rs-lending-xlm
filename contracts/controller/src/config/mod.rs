//! Owner-gated protocol configuration: registries, hub/spoke listings, limits,
//! and allowlists. Auth is `#[only_owner]` (governance after execute; GUARDIAN
//! immediate for hub/spoke create and tighten-only flags). See
//! [ADR 0010](../../../architecture/decisions/0010-governance-timelock-for-controller-admin.md)
//! and [INVARIANTS](../../../architecture/INVARIANTS.md) §5.1.

mod approvals;
mod asset;
mod hub;
mod limits;
mod registry;
mod spoke;

#[cfg(feature = "certora")]
pub(crate) use asset::{add_asset_to_spoke, edit_asset_in_spoke};
pub(crate) use hub::require_hub_active;
#[cfg(feature = "certora")]
pub(crate) use spoke::remove_spoke;

#[cfg(test)]
use common::types::HubConfig;
use common::types::{HubAssetKey, PositionLimits, PositionManagerConfig, SpokeAssetArgs};
use soroban_sdk::{contractimpl, Address, BytesN, Env};
use stellar_macros::only_owner;

use crate::{storage, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    /// Sets the swap aggregator used by strategy swaps. Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdateSwapAggregatorEvent` — new aggregator address.
    #[only_owner]
    pub fn set_swap_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_swap_aggregator(&env, addr);
    }

    /// Sets the price aggregator (oracle authority). Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdatePriceAggregatorEvent` — new aggregator address.
    #[only_owner]
    pub fn set_price_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_price_aggregator(&env, addr);
    }

    /// Returns the wired price aggregator.
    ///
    /// # Errors
    /// * `AggregatorNotSet` — no aggregator configured.
    pub fn price_aggregator(env: Env) -> Address {
        storage::get_price_aggregator(&env)
    }

    /// Sets the revenue accumulator for `claim_revenue`. Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdateAccumulatorEvent` — new accumulator address.
    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_accumulator(&env, addr);
    }

    /// Sets the pool Wasm template hash for `deploy_pool`. Owner only (gov timelock).
    ///
    /// # Events
    /// * `UpdatePoolTemplateEvent` — new template hash.
    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        registry::set_liquidity_pool_template(&env, hash);
    }

    /// Sets per-account max supply/borrow position counts. Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidPositionLimits` — a side is outside `1..=POSITION_LIMIT_MAX`.
    ///
    /// # Events
    /// * `UpdatePositionLimitsEvent` — new caps.
    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        limits::set_position_limits(&env, limits);
    }

    /// Sets the min LTV-weighted collateral while debt is open (USD WAD ≥ 0).
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `InvalidBorrowParams` — `floor_wad` is negative.
    ///
    /// # Events
    /// * `UpdateMinBorrowCollateralEvent` — new floor.
    #[only_owner]
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        limits::set_min_borrow_collateral_usd(&env, floor_wad);
    }

    /// Returns the min-borrow-collateral floor (USD WAD).
    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    /// Creates an active hub; inert until markets list. Owner only (gov;
    /// GUARDIAN immediate path).
    ///
    /// # Events
    /// * `CreateHubEvent` — new hub id.
    #[only_owner]
    pub fn create_hub(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        hub::create_hub(&env)
    }

    /// Creates a spoke with default liquidation curve. Owner only (gov;
    /// GUARDIAN immediate path).
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — new spoke snapshot.
    #[only_owner]
    pub fn add_spoke(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        spoke::add_spoke(&env)
    }

    /// Deprecates a spoke (gates subsequent spoke reads). Owner only (gov timelock).
    ///
    /// # Errors
    /// * `SpokeNotFound` — unknown `id`.
    /// * `SpokeDeprecated` — already deprecated.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — deprecated snapshot.
    #[only_owner]
    pub fn remove_spoke(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        spoke::remove_spoke(&env, id);
    }

    /// Sets the spoke liquidation curve (target HF, knee, factor ≤ BPS).
    /// Owner only (gov timelock).
    ///
    /// # Errors
    /// * `SpokeNotFound` — unknown `id`.
    /// * `InvalidLiquidationCurve` — bounds violated.
    ///
    /// # Events
    /// * `UpdateSpokeEvent` — updated snapshot.
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
    #[only_owner]
    pub fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::add_asset_to_spoke(&env, &input);
    }

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
    #[only_owner]
    pub fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::edit_asset_in_spoke(&env, &input);
    }

    /// Tightens `paused`/`frozen` on a listing (clearing a flag reverts). Owner
    /// only (gov; GUARDIAN immediate path).
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    /// * `SpokeAssetFlagRelaxation` — a flag would clear (`true → false`).
    ///
    /// # Events
    /// * `UpdateSpokeAssetEvent` — updated listing.
    #[only_owner]
    pub fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        storage::renew_controller_instance(&env);
        asset::set_spoke_asset_flags(&env, spoke_id, hub_asset, paused, frozen);
    }

    /// Unlists a hub-asset when spoke usage is zero. Owner only (gov timelock).
    ///
    /// # Errors
    /// * `AssetNotInSpoke` — listing missing.
    /// * `SpokeAssetInUse` — scaled supply or borrow usage is non-zero.
    ///
    /// # Events
    /// * `RemoveSpokeAssetEvent` — removed listing key.
    #[only_owner]
    pub fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        asset::remove_asset_from_spoke(&env, hub_asset, spoke_id);
    }

    /// Returns whether `pool` is on the Blend migration allowlist.
    pub fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        approvals::is_blend_pool_approved(&env, pool)
    }

    /// Allows a Blend pool as a migration source. Owner only (gov timelock).
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` — `approved = true`.
    #[only_owner]
    pub fn approve_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, true);
    }

    /// Revokes a Blend migration allowlist entry. Owner only (gov timelock).
    ///
    /// # Events
    /// * `ApproveBlendPoolEvent` — `approved = false`.
    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, false);
    }

    /// Registers or removes a position manager (`false` deletes). Owner only
    /// (gov timelock).
    #[only_owner]
    pub fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        storage::renew_controller_instance(&env);
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active });
    }
}

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
