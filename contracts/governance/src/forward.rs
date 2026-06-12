//! Owner- and ORACLE-gated admin entrypoints.
//!
//! Every entrypoint validates its inputs, then forwards to the controller's
//! thin owner-gated setters through `ControllerAdminClient`. Invoker auth
//! makes the controller accept governance (its owner) as the direct
//! cross-contract caller. The controller keeps state-dependent invariants
//! and event emission; governance owns all input validation.

use common::errors::{GenericError, OracleError};
use common::types::{InterestRateModel, MarketParamsRaw};
use controller_interface::types::{AssetConfigRaw, MarketOracleConfigInput, PositionLimits};
use controller_interface::ControllerAdminClient;
use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol};
use stellar_macros::{only_owner, only_role};

use crate::{storage, validate, Governance, GovernanceArgs, GovernanceClient};

fn controller_client(env: &Env) -> ControllerAdminClient<'_> {
    ControllerAdminClient::new(env, &storage::get_controller(env))
}

#[contractimpl]
impl Governance {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        validate::require_contract_address(&env, &addr, OracleError::InvalidAggregator);
        controller_client(&env).set_aggregator(&addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        validate::require_contract_address(&env, &addr, GenericError::NotSmartContract);
        controller_client(&env).set_accumulator(&addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        validate::require_nonzero_wasm_hash(&env, &hash);
        controller_client(&env).set_liquidity_pool_template(&hash);
    }

    #[only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfigRaw) {
        validate::asset::validate_asset_config(&env, &cfg);
        controller_client(&env).edit_asset_config(&asset, &cfg);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        validate::asset::validate_position_limits(&env, &limits);
        controller_client(&env).set_position_limits(&limits);
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
        validate::asset::validate_risk_bounds(&env, ltv, threshold, bonus);
        controller_client(&env).add_e_mode_category(&ltv, &threshold, &bonus)
    }

    #[only_owner]
    pub fn edit_e_mode_category(env: Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
        validate::asset::validate_risk_bounds(&env, ltv, threshold, bonus);
        controller_client(&env).edit_e_mode_category(&id, &ltv, &threshold, &bonus);
    }

    #[only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        controller_client(&env).remove_e_mode_category(&id);
    }

    #[only_owner]
    pub fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        controller_client(&env).add_asset_to_e_mode_category(
            &asset,
            &category_id,
            &can_collateral,
            &can_borrow,
        );
    }

    #[only_owner]
    pub fn edit_asset_in_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        controller_client(&env).edit_asset_in_e_mode_category(
            &asset,
            &category_id,
            &can_collateral,
            &can_borrow,
        );
    }

    #[only_owner]
    pub fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32) {
        controller_client(&env).remove_asset_from_e_mode(&asset, &category_id);
    }

    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        controller_client(&env).approve_token(&token);
    }

    #[only_owner]
    pub fn revoke_token(env: Env, token: Address) {
        controller_client(&env).revoke_token(&token);
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
    ) -> Address {
        let token_decimals = validate::asset::validate_and_fetch_token_decimals(&env, &asset);
        validate::asset::validate_market_creation(&env, &asset, &params, &config, token_decimals);
        controller_client(&env).create_liquidity_pool(&asset, &params, &config)
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        // The pool re-validates; validating here reverts early with precise errors.
        params.verify(&env);
        controller_client(&env).upgrade_liquidity_pool_params(&asset, &params);
    }

    #[only_owner]
    pub fn deploy_pool(env: Env) -> Address {
        controller_client(&env).deploy_pool()
    }

    #[only_owner]
    pub fn upgrade_pool(env: Env, new_wasm_hash: BytesN<32>) {
        controller_client(&env).upgrade_pool(&new_wasm_hash);
    }

    #[only_owner]
    pub fn pause(env: Env) {
        controller_client(&env).pause();
    }

    #[only_owner]
    pub fn unpause(env: Env) {
        controller_client(&env).unpause();
    }

    #[only_owner]
    pub fn grant_controller_role(env: Env, account: Address, role: Symbol) {
        controller_client(&env).grant_role(&account, &role);
    }

    #[only_owner]
    pub fn revoke_controller_role(env: Env, account: Address, role: Symbol) {
        controller_client(&env).revoke_role(&account, &role);
    }

    #[only_owner]
    pub fn upgrade_controller(env: Env, new_wasm_hash: BytesN<32>) {
        controller_client(&env).upgrade(&new_wasm_hash);
    }

    #[only_owner]
    pub fn migrate_controller(env: Env, new_version: u32) {
        controller_client(&env).migrate(&new_version);
    }

    #[only_owner]
    pub fn transfer_controller_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        controller_client(&env).transfer_ownership(&new_owner, &live_until_ledger);
    }

    #[only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        let tolerance = validate::tolerance::validate_and_calculate_tolerances(
            &env,
            cfg.first_tolerance_bps,
            cfg.last_tolerance_bps,
        );
        let controller = storage::get_controller(&env);
        let config = validate::oracle_probe::validate_market_oracle_sources(
            &env,
            &controller,
            &asset,
            &cfg,
            tolerance,
        );
        ControllerAdminClient::new(&env, &controller).set_market_oracle_config(&asset, &config);
    }

    #[only_role(caller, "ORACLE")]
    pub fn edit_oracle_tolerance(
        env: Env,
        caller: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
    ) {
        let tolerance = validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        );
        controller_client(&env).set_oracle_tolerance(&asset, &tolerance);
    }
}
