//! Client-only ABI mirror of the controller's admin entrypoints.
//!
//! `#[contractclient]` generates `ControllerAdminClient` for governance
//! forwarding. Mirrors the deployed controller-admin entrypoints 1:1 by ABI name.

use common::types::{
    HubAssetKey, MarketOracleConfig, OraclePriceFluctuation, PositionLimits, SpokeAssetArgs,
    SpokeAssetConfig,
};
use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractclient, Address, BytesN, Env};

/// Mirrors the controller admin ABI for governance forwarding.
#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    /// Sets the aggregator (swap router) address.
    fn set_aggregator(env: Env, addr: Address);
    /// Sets the rewards accumulator address.
    fn set_accumulator(env: Env, addr: Address);
    /// Sets the liquidity-pool wasm template hash.
    fn set_liquidity_pool_template(env: Env, hash: BytesN<32>);
    /// Sets the per-account position limits.
    fn set_position_limits(env: Env, limits: PositionLimits);
    /// Sets the instance-wide minimum borrow collateral floor (USD WAD).
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);
    /// Creates a new hub and returns its id.
    fn create_hub(env: Env) -> u32;
    /// Creates a new spoke and returns its id.
    fn add_spoke(env: Env) -> u32;
    /// Removes the spoke with `id`.
    fn remove_spoke(env: Env, id: u32);
    /// Overrides a spoke's liquidation curve (target HF, HF for max bonus,
    /// bonus factor).
    fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    );
    /// Lists an asset on a spoke with its risk config.
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs);
    /// Updates the risk config of an asset already listed on a spoke.
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs);
    /// Delists an asset from a spoke.
    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32);
    /// Adds `token` to the supported-token allow-list.
    fn approve_token(env: Env, token: Address);
    /// Removes `token` from the supported-token allow-list.
    fn revoke_token(env: Env, token: Address);
    /// Adds `pool` to the Blend-pool migration allow-list.
    fn approve_blend_pool(env: Env, pool: Address);
    /// Removes `pool` from the Blend-pool migration allow-list.
    fn revoke_blend_pool(env: Env, pool: Address);
    /// Sets the oracle configuration for a hub-asset market.
    fn set_market_oracle_config(env: Env, hub_asset: HubAssetKey, config: MarketOracleConfig);
    /// Sets the price-fluctuation tolerance for `asset`.
    fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation);
    /// Disables the oracle for `asset`.
    fn disable_token_oracle(env: Env, asset: Address);
    /// Registers or deactivates `manager` as a position manager.
    fn set_position_manager(env: Env, manager: Address, is_active: bool);
    /// Creates a hub-asset market and returns its pool address.
    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address;
    /// Replaces the interest-rate model for a hub-asset market.
    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel);
    /// Deploys the central liquidity pool and returns its address.
    fn deploy_pool(env: Env) -> Address;
    /// Upgrades the central liquidity pool to `new_wasm_hash`.
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);
    /// Pauses the controller.
    fn pause(env: Env);
    /// Resumes the controller.
    fn unpause(env: Env);
    /// Upgrades the controller to `new_wasm_hash`.
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    /// Runs the migration hook for `new_version`.
    fn migrate(env: Env, new_version: u32);
    /// Starts a two-step ownership transfer of the controller.
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);
    /// Per-spoke risk listing read-back; each spoke (id `>= 1`) holds its own config.
    fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig;
}
