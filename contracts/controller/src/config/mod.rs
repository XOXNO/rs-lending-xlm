//! Owner-gated protocol configuration entrypoints.

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
    /// Swap aggregator (strategy swap venue). Owner = governance timelock.
    #[only_owner]
    pub fn set_swap_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_swap_aggregator(&env, addr);
    }

    /// Price aggregator (oracle authority). Owner = governance timelock.
    #[only_owner]
    pub fn set_price_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_price_aggregator(&env, addr);
    }

    /// Revenue accumulator. Owner = governance timelock.
    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        registry::set_accumulator(&env, addr);
    }

    /// Pool WASM template hash. Owner = governance timelock.
    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        registry::set_liquidity_pool_template(&env, hash);
    }

    /// Per-account max supply/borrow positions. Owner = governance timelock.
    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        limits::set_position_limits(&env, limits);
    }

    /// Min LTV-weighted collateral while debt open (USD WAD ≥ 0). Owner = timelock.
    #[only_owner]
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        limits::set_min_borrow_collateral_usd(&env, floor_wad);
    }

    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    /// Create hub; inert until markets listed. Owner = gov (timelock or GUARDIAN).
    #[only_owner]
    pub fn create_hub(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        hub::create_hub(&env)
    }

    /// Create spoke with default liq curve. Owner = gov (timelock or GUARDIAN).
    #[only_owner]
    pub fn add_spoke(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        spoke::add_spoke(&env)
    }

    /// Deprecate spoke (gates reads). Owner = governance timelock.
    #[only_owner]
    pub fn remove_spoke(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        spoke::remove_spoke(&env, id);
    }

    /// Spoke liq curve (target HF, knee, factor ≤ BPS). Owner = timelock.
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

    /// List hub-asset on spoke; pool market must exist. Owner = timelock.
    #[only_owner]
    pub fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::add_asset_to_spoke(&env, &input);
    }

    /// Edit listing (ok on deprecated; caps may sit below usage). Owner = timelock.
    #[only_owner]
    pub fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::edit_asset_in_spoke(&env, &input);
    }

    /// Pause/freeze only; tighten-only. GUARDIAN immediate path.
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

    /// Unlist when usage is zero (freeze + drain first). Owner = timelock.
    #[only_owner]
    pub fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        asset::remove_asset_from_spoke(&env, hub_asset, spoke_id);
    }

    pub fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        approvals::is_blend_pool_approved(&env, pool)
    }

    /// Allow Blend pool as migration source. Owner = timelock.
    #[only_owner]
    pub fn approve_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, true);
    }

    /// Revoke Blend migration allowlist entry. Owner = timelock.
    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, false);
    }

    /// Register/remove position manager (`false` deletes). Owner = timelock.
    #[only_owner]
    pub fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        storage::renew_controller_instance(&env);
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active });
    }
}

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
