//! Owner- and role-gated protocol configuration entrypoints.

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
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_aggregator(&env, addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_accumulator(&env, addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        approvals::set_liquidity_pool_template(&env, hash);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        limits::set_position_limits(&env, limits);
    }

    #[only_owner]
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        limits::set_min_borrow_collateral_usd(&env, floor_wad);
    }

    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        limits::get_min_borrow_collateral_usd(&env)
    }

    #[only_owner]
    pub fn create_hub(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        hub::create_hub(&env)
    }

    #[only_owner]
    pub fn add_spoke(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        spoke::add_spoke(&env)
    }

    #[only_owner]
    pub fn remove_spoke(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        spoke::remove_spoke(&env, id);
    }

    #[only_owner]
    pub fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::add_asset_to_spoke(&env, &input);
    }

    #[only_owner]
    pub fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        asset::edit_asset_in_spoke(&env, &input);
    }

    #[only_owner]
    pub fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        asset::remove_asset_from_spoke(&env, hub_asset, spoke_id);
    }

    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        approvals::set_token_approval(&env, token, true);
    }

    #[only_owner]
    pub fn revoke_token(env: Env, token: Address) {
        approvals::set_token_approval(&env, token, false);
    }

    pub fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        approvals::is_blend_pool_approved(&env, pool)
    }

    #[only_owner]
    pub fn approve_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, true);
    }

    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        approvals::set_blend_pool_approval(&env, pool, false);
    }

    #[only_owner]
    pub fn set_market_oracle_config(env: Env, hub_asset: HubAssetKey, config: MarketOracleConfig) {
        storage::renew_controller_instance(&env);
        oracle::set_market_oracle_config(&env, hub_asset, config);
    }

    #[only_owner]
    pub fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation) {
        storage::renew_controller_instance(&env);
        oracle::set_oracle_tolerance(&env, asset, tolerance);
    }

    #[only_owner]
    pub fn disable_token_oracle(env: Env, asset: Address) {
        storage::renew_controller_instance(&env);
        oracle::disable_token_oracle(&env, asset);
    }

    #[only_owner]
    pub fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        storage::renew_controller_instance(&env);
        position_manager::set_position_manager(&env, manager, is_active);
    }
}

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
