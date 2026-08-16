use common::types::{
    HubAssetKey, InterestRateModel, MarketParamsRaw, PositionLimits, SpokeAssetArgs,
};
use soroban_sdk::{contractclient, Address, BytesN, Env, String};

#[contractclient(name = "ControllerAdminClient")]
pub trait ControllerAdmin {
    fn set_swap_aggregator(env: Env, addr: Address);

    fn set_price_aggregator(env: Env, addr: Address);

    fn set_accumulator(env: Env, addr: Address);

    fn set_position_limits(env: Env, limits: PositionLimits);

    fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128);

    fn set_position_manager(env: Env, manager: Address, is_active: bool);

    fn approve_blend_pool(env: Env, pool: Address);

    fn revoke_blend_pool(env: Env, pool: Address);

    fn create_hub(env: Env) -> u32;

    fn add_spoke(env: Env) -> u32;

    fn remove_spoke(env: Env, id: u32);

    fn set_spoke_liquidation_curve(
        env: Env,
        id: u32,
        target_hf_wad: i128,
        hf_for_max_bonus_wad: i128,
        liquidation_bonus_factor_bps: u32,
    );

    fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs);

    fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs);

    fn set_spoke_asset_flags(
        env: Env,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
        no_seize: bool,
    );

    fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32);

    fn deploy_pool(env: Env, wasm_hash: BytesN<32>) -> Address;

    fn deploy_position_nft(
        env: Env,
        wasm_hash: BytesN<32>,
        uri: String,
        name: String,
        symbol: String,
    ) -> Address;

    fn create_liquidity_pool(
        env: Env,
        hub_id: u32,
        asset: Address,
        params: MarketParamsRaw,
    ) -> Address;

    fn upgrade_liquidity_pool_params(env: Env, hub_asset: HubAssetKey, params: InterestRateModel);

    fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>);

    fn force_socialize_bad_debt(env: Env, account_id: u64);

    fn pause(env: Env);

    fn unpause(env: Env);

    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);

    fn migrate(env: Env, new_version: u32);

    fn get_app_version(env: Env) -> u32;

    fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32);

    fn accept_ownership(env: Env);
}
