//! Client-only ABI mirror of the controller's admin entrypoints.
//!
//! `#[contractclient]` generates `ControllerAdminClient` for governance
//! forwarding. Matches deployed controller-admin entrypoints by ABI name.

use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PositionLimits, SpokeAssetArgs,
    SpokeAssetConfig,
};
use soroban_sdk::{contractclient, Address, BytesN, Env};

/// Mirrors the controller admin ABI for governance forwarding.
#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    // --- wiring ---

    fn set_swap_aggregator(env: Env, addr: Address);
    fn set_price_aggregator(env: Env, addr: Address);
    fn set_accumulator(env: Env, addr: Address);
    fn set_liquidity_pool_template(env: Env, hash: BytesN<32>);
    fn set_position_limits(env: Env, limits: PositionLimits);
    /// Instance-wide minimum borrow collateral (USD WAD).
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);

    // --- hubs / spokes / listings ---

    fn create_hub(env: Env) -> u32;
    fn add_spoke(env: Env) -> u32;
    fn remove_spoke(env: Env, id: u32);
    /// Target HF, HF for max bonus, bonus factor (per spoke).
    fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    );
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs);
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs);
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32);
    /// Paused / frozen flags on an existing spoke listing.
    fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    );
    /// Per-spoke risk listing (`spoke_id >= 1`).
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;

    // --- integrations ---

    fn approve_blend_pool(env: Env, pool: Address);
    fn revoke_blend_pool(env: Env, pool: Address);
    fn set_position_manager(env: Env, manager: Address, is_active: bool);

    // --- pool ---

    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address;
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel);
    /// Deploys the central liquidity pool; returns its address.
    fn deploy_pool(env: Env) -> Address;
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);

    // --- lifecycle ---

    fn pause(env: Env);
    fn unpause(env: Env);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn migrate(env: Env, new_version: u32);
    /// Starts two-step ownership transfer of the controller.
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);
}
