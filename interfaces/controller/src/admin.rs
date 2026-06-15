use crate::types::{
    AssetConfigRaw, MarketConfig, MarketOracleConfig, OraclePriceFluctuation, PositionLimits,
};
use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractclient, Address, BytesN, Env, Symbol};

/// Mirrors the controller admin ABI for governance forwarding.
#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    fn set_aggregator(env: Env, addr: Address);
    fn set_accumulator(env: Env, addr: Address);
    fn set_liquidity_pool_template(env: Env, hash: BytesN<32>);
    fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfigRaw);
    fn set_position_limits(env: Env, limits: PositionLimits);
    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);
    fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32;
    fn edit_e_mode_category(env: Env, id: u32, ltv: u32, threshold: u32, bonus: u32);
    fn remove_e_mode_category(env: Env, id: u32);
    fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    );
    fn edit_asset_in_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    );
    fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32);
    fn approve_token(env: Env, token: Address);
    fn revoke_token(env: Env, token: Address);
    fn set_market_oracle_config(env: Env, asset: Address, config: MarketOracleConfig);
    fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation);
    fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
    ) -> Address;
    fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel);
    fn deploy_pool(env: Env) -> Address;
    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);
    fn pause(env: Env);
    fn unpause(env: Env);
    fn grant_role(env: Env, account: Address, role: Symbol);
    fn revoke_role(env: Env, account: Address, role: Symbol);
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);
    fn migrate(env: Env, new_version: u32);
    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);
    /// Read-back used by governance oracle validation (quote-market checks).
    fn get_market_config(env: Env, asset: Address) -> MarketConfig;
}
