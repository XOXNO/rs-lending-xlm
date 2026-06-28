use crate::types::{
    MarketOracleConfig, OraclePriceFluctuation, PositionLimits, SpokeAssetArgs, SpokeAssetConfig,
};
use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractclient, Address, BytesN, Env};

/// Mirrors the controller admin ABI for governance forwarding.
#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    fn set_aggregator(env: Env, addr: Address);
    fn set_accumulator(env: Env, addr: Address);
    fn set_liquidity_pool_template(env: Env, hash: BytesN<32>);
    fn set_position_limits(env: Env, limits: PositionLimits);
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);
    fn add_spoke(env: Env) -> u32;
    fn remove_spoke(env: Env, id: u32);
    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs);
    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs);
    fn remove_asset_from_spoke(env: Env, asset: Address, spoke_id: u32);
    fn approve_token(env: Env, token: Address);
    fn revoke_token(env: Env, token: Address);
    fn approve_blend_pool(env: Env, pool: Address);
    fn revoke_blend_pool(env: Env, pool: Address);
    fn set_market_oracle_config(env: Env, asset: Address, config: MarketOracleConfig);
    fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation);
    fn disable_token_oracle(env: Env, asset: Address);
    fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: SpokeAssetConfig,
    ) -> Address;
    fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel);
    fn update_pool_caps(env: Env, asset: Address, supply_cap: i128, borrow_cap: i128);
    fn deploy_pool(env: Env) -> Address;
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);
    fn pause(env: Env);
    fn unpause(env: Env);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn migrate(env: Env, new_version: u32);
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);
    /// Per-spoke risk listing read-back (spoke 0 is the general base listing).
    fn get_spoke_asset(env: Env, spoke_id: u32, asset: Address) -> SpokeAssetConfig;
}
