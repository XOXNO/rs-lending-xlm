//! Timelocked controller-admin proposers and the immediate pause forwarders.
//!
//! Every protocol-affecting controller-admin op is reachable only by scheduling
//! it through a typed `propose_*` proposer: the proposer holds PROPOSER, runs
//! the FULL input validation now, builds an `Operation` targeting the
//! controller's thin owner-gated setter, and schedules it at the minimum delay.
//! Execution happens later through `execute` (see `timelock.rs`), which invokes
//! the controller as a governance->controller cross-call authorized by
//! governance's ownership. The generic OZ `schedule` is NOT exposed, so nothing
//! unvalidated can ever be queued.
//!
//! `pause`/`unpause` stay immediate emergency brakes (owner-gated), since a 48h
//! delay on halting a compromised market is unacceptable.
//!
//! The testing block mirrors the old immediate forwarders so the harness builder
//! can configure markets in one frame; it is excluded from the production wasm.

use common::errors::{GenericError, OracleError};
use common::types::{InterestRateModel, MarketParamsRaw};
use controller_interface::types::{AssetConfigRaw, MarketOracleConfigInput, PositionLimits};
use controller_interface::ControllerAdminClient;
use soroban_sdk::{
    assert_with_error, contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val,
};
use stellar_access::access_control;
use stellar_governance::timelock::{get_min_delay, schedule_operation, Operation};
use stellar_macros::only_owner;
#[cfg(any(test, feature = "testing"))]
use stellar_macros::only_role;

use crate::access::PROPOSER_ROLE;
use crate::{storage, validate, Governance, GovernanceArgs, GovernanceClient};

fn controller_client(env: &Env) -> ControllerAdminClient<'_> {
    ControllerAdminClient::new(env, &storage::get_controller(env))
}

/// Authorizes a proposer and renews the instance TTL. Every `propose_*` shares
/// this preamble before running its op-specific validation.
fn begin_proposal(env: &Env, proposer: &Address) {
    storage::renew_governance_instance(env);
    proposer.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, PROPOSER_ROLE), proposer);
}

/// Schedules a validated controller-targeted operation at the minimum delay.
fn schedule_controller_op(
    env: &Env,
    function: &str,
    args: soroban_sdk::Vec<Val>,
    salt: BytesN<32>,
) -> BytesN<32> {
    let operation = Operation {
        target: storage::get_controller(env),
        function: Symbol::new(env, function),
        args,
        predecessor: BytesN::from_array(env, &[0u8; 32]),
        salt,
    };
    schedule_controller(env, &operation)
}

fn schedule_controller(env: &Env, operation: &Operation) -> BytesN<32> {
    schedule_operation(env, operation, get_min_delay(env))
}

fn require_known_controller_role(env: &Env, role: &Symbol) {
    let keeper = Symbol::new(env, "KEEPER");
    let revenue = Symbol::new(env, "REVENUE");
    let oracle = Symbol::new(env, "ORACLE");
    assert_with_error!(
        env,
        role == &keeper || role == &revenue || role == &oracle,
        GenericError::InvalidRole
    );
}

/// Validates and probes a market oracle input into the resolved
/// `MarketOracleConfig` that `set_market_oracle_config` persists. Shared by the
/// `propose_configure_market_oracle` proposer and the `resolve_market_oracle_config`
/// view so the view returns byte-identical output to what the proposer schedules.
pub(crate) fn resolve_market_oracle(
    env: &Env,
    asset: &Address,
    cfg: &MarketOracleConfigInput,
) -> controller_interface::types::MarketOracleConfig {
    let tolerance = validate::tolerance::validate_and_calculate_tolerances(
        env,
        cfg.first_tolerance_bps,
        cfg.last_tolerance_bps,
    );
    let controller = storage::get_controller(env);
    validate::oracle_probe::validate_market_oracle_sources(env, &controller, asset, cfg, tolerance)
}

#[contractimpl]
impl Governance {
    pub fn propose_set_aggregator(
        env: Env,
        proposer: Address,
        addr: Address,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::require_contract_address(&env, &addr, OracleError::InvalidAggregator);
        schedule_controller_op(
            &env,
            "set_aggregator",
            vec![&env, addr.into_val(&env)],
            salt,
        )
    }

    pub fn propose_set_accumulator(
        env: Env,
        proposer: Address,
        addr: Address,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::require_contract_address(&env, &addr, GenericError::NotSmartContract);
        schedule_controller_op(
            &env,
            "set_accumulator",
            vec![&env, addr.into_val(&env)],
            salt,
        )
    }

    pub fn propose_set_pool_template(
        env: Env,
        proposer: Address,
        hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::require_nonzero_wasm_hash(&env, &hash);
        schedule_controller_op(
            &env,
            "set_liquidity_pool_template",
            vec![&env, hash.into_val(&env)],
            salt,
        )
    }

    pub fn propose_edit_asset_config(
        env: Env,
        proposer: Address,
        asset: Address,
        cfg: AssetConfigRaw,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::asset::validate_asset_config(&env, &cfg);
        schedule_controller_op(
            &env,
            "edit_asset_config",
            vec![&env, asset.into_val(&env), cfg.into_val(&env)],
            salt,
        )
    }

    pub fn propose_set_position_limits(
        env: Env,
        proposer: Address,
        limits: PositionLimits,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::asset::validate_position_limits(&env, &limits);
        schedule_controller_op(
            &env,
            "set_position_limits",
            vec![&env, limits.into_val(&env)],
            salt,
        )
    }

    pub fn propose_add_e_mode_category(
        env: Env,
        proposer: Address,
        ltv: u32,
        threshold: u32,
        bonus: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::asset::validate_risk_bounds(&env, ltv, threshold, bonus);
        schedule_controller_op(
            &env,
            "add_e_mode_category",
            vec![
                &env,
                ltv.into_val(&env),
                threshold.into_val(&env),
                bonus.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_edit_e_mode_category(
        env: Env,
        proposer: Address,
        id: u32,
        ltv: u32,
        threshold: u32,
        bonus: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::asset::validate_risk_bounds(&env, ltv, threshold, bonus);
        schedule_controller_op(
            &env,
            "edit_e_mode_category",
            vec![
                &env,
                id.into_val(&env),
                ltv.into_val(&env),
                threshold.into_val(&env),
                bonus.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_remove_e_mode_category(
        env: Env,
        proposer: Address,
        id: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "remove_e_mode_category",
            vec![&env, id.into_val(&env)],
            salt,
        )
    }

    pub fn propose_add_asset_to_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "add_asset_to_e_mode_category",
            vec![
                &env,
                asset.into_val(&env),
                category_id.into_val(&env),
                can_collateral.into_val(&env),
                can_borrow.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_edit_asset_in_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "edit_asset_in_e_mode_category",
            vec![
                &env,
                asset.into_val(&env),
                category_id.into_val(&env),
                can_collateral.into_val(&env),
                can_borrow.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_remove_asset_from_e_mode(
        env: Env,
        proposer: Address,
        asset: Address,
        category_id: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "remove_asset_from_e_mode",
            vec![&env, asset.into_val(&env), category_id.into_val(&env)],
            salt,
        )
    }

    pub fn propose_approve_token(
        env: Env,
        proposer: Address,
        token: Address,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "approve_token",
            vec![&env, token.into_val(&env)],
            salt,
        )
    }

    pub fn propose_revoke_token(
        env: Env,
        proposer: Address,
        token: Address,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(&env, "revoke_token", vec![&env, token.into_val(&env)], salt)
    }

    pub fn propose_create_liquidity_pool(
        env: Env,
        proposer: Address,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        let token_decimals = validate::asset::validate_and_fetch_token_decimals(&env, &asset);
        validate::asset::validate_market_creation(&env, &asset, &params, &config, token_decimals);
        schedule_controller_op(
            &env,
            "create_liquidity_pool",
            vec![
                &env,
                asset.into_val(&env),
                params.into_val(&env),
                config.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_upgrade_pool_params(
        env: Env,
        proposer: Address,
        asset: Address,
        params: InterestRateModel,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        // The pool re-validates; validating here reverts early with precise errors.
        params.verify(&env);
        schedule_controller_op(
            &env,
            "upgrade_liquidity_pool_params",
            vec![&env, asset.into_val(&env), params.into_val(&env)],
            salt,
        )
    }

    pub fn propose_deploy_pool(env: Env, proposer: Address, salt: BytesN<32>) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(&env, "deploy_pool", vec![&env], salt)
    }

    pub fn propose_upgrade_pool(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "upgrade_pool",
            vec![&env, new_wasm_hash.into_val(&env)],
            salt,
        )
    }

    pub fn propose_grant_controller_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        require_known_controller_role(&env, &role);
        schedule_controller_op(
            &env,
            "grant_role",
            vec![&env, account.into_val(&env), role.into_val(&env)],
            salt,
        )
    }

    pub fn propose_revoke_controller_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        require_known_controller_role(&env, &role);
        schedule_controller_op(
            &env,
            "revoke_role",
            vec![&env, account.into_val(&env), role.into_val(&env)],
            salt,
        )
    }

    pub fn propose_upgrade_controller(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "upgrade",
            vec![&env, new_wasm_hash.into_val(&env)],
            salt,
        )
    }

    pub fn propose_migrate_controller(
        env: Env,
        proposer: Address,
        new_version: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_controller_op(
            &env,
            "migrate",
            vec![&env, new_version.into_val(&env)],
            salt,
        )
    }

    pub fn propose_transfer_ctrl_ownership(
        env: Env,
        proposer: Address,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate::require_contract_address(&env, &new_owner, GenericError::NotSmartContract);
        schedule_controller_op(
            &env,
            "transfer_ownership",
            vec![
                &env,
                new_owner.into_val(&env),
                live_until_ledger.into_val(&env),
            ],
            salt,
        )
    }

    pub fn propose_configure_market_oracle(
        env: Env,
        proposer: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        let config = resolve_market_oracle(&env, &asset, &cfg);
        schedule_controller_op(
            &env,
            "set_market_oracle_config",
            vec![&env, asset.into_val(&env), config.into_val(&env)],
            salt,
        )
    }

    pub fn propose_edit_oracle_tolerance(
        env: Env,
        proposer: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        let tolerance = validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        );
        schedule_controller_op(
            &env,
            "set_oracle_tolerance",
            vec![&env, asset.into_val(&env), tolerance.into_val(&env)],
            salt,
        )
    }
}

#[contractimpl]
impl Governance {
    /// Emergency brake: halt the controller immediately, owner-gated. A timelock
    /// delay on pausing a compromised market is unacceptable, so this is not
    /// scheduled.
    #[only_owner]
    pub fn pause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).pause();
    }

    /// Resume the controller, owner-gated and immediate (the inverse brake).
    #[only_owner]
    pub fn unpause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).unpause();
    }
}

/// Immediate owner-gated forwarders used only by the test harness builder so it
/// can configure markets in a single frame. Excluded from the production wasm,
/// where the same operations are reachable only through the timelocked
/// `propose_*` proposers above. Same precedent as the testing-only
/// `set_controller`.
#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        storage::renew_governance_instance(&env);
        validate::require_contract_address(&env, &addr, OracleError::InvalidAggregator);
        controller_client(&env).set_aggregator(&addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_governance_instance(&env);
        validate::require_contract_address(&env, &addr, GenericError::NotSmartContract);
        controller_client(&env).set_accumulator(&addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_governance_instance(&env);
        validate::require_nonzero_wasm_hash(&env, &hash);
        controller_client(&env).set_liquidity_pool_template(&hash);
    }

    #[only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfigRaw) {
        storage::renew_governance_instance(&env);
        validate::asset::validate_asset_config(&env, &cfg);
        controller_client(&env).edit_asset_config(&asset, &cfg);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_governance_instance(&env);
        validate::asset::validate_position_limits(&env, &limits);
        controller_client(&env).set_position_limits(&limits);
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
        storage::renew_governance_instance(&env);
        validate::asset::validate_risk_bounds(&env, ltv, threshold, bonus);
        controller_client(&env).add_e_mode_category(&ltv, &threshold, &bonus)
    }

    #[only_owner]
    pub fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        storage::renew_governance_instance(&env);
        controller_client(&env).add_asset_to_e_mode_category(
            &asset,
            &category_id,
            &can_collateral,
            &can_borrow,
        );
    }

    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        storage::renew_governance_instance(&env);
        controller_client(&env).approve_token(&token);
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParamsRaw,
        config: AssetConfigRaw,
    ) -> Address {
        storage::renew_governance_instance(&env);
        let token_decimals = validate::asset::validate_and_fetch_token_decimals(&env, &asset);
        validate::asset::validate_market_creation(&env, &asset, &params, &config, token_decimals);
        controller_client(&env).create_liquidity_pool(&asset, &params, &config)
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        storage::renew_governance_instance(&env);
        // The pool re-validates; validating here reverts early with precise errors.
        params.verify(&env);
        controller_client(&env).upgrade_liquidity_pool_params(&asset, &params);
    }

    #[only_owner]
    pub fn deploy_pool(env: Env) -> Address {
        storage::renew_governance_instance(&env);
        controller_client(&env).deploy_pool()
    }

    #[only_owner]
    pub fn grant_controller_role(env: Env, account: Address, role: Symbol) {
        storage::renew_governance_instance(&env);
        controller_client(&env).grant_role(&account, &role);
    }

    #[only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        storage::renew_governance_instance(&env);
        let tolerance = validate::tolerance::validate_and_calculate_tolerances(
            &env,
            cfg.first_tolerance_bps,
            cfg.last_tolerance_bps,
        );
        let client = controller_client(&env);
        let config = validate::oracle_probe::validate_market_oracle_sources(
            &env,
            &client.address,
            &asset,
            &cfg,
            tolerance,
        );
        client.set_market_oracle_config(&asset, &config);
    }

    #[only_role(caller, "ORACLE")]
    pub fn edit_oracle_tolerance(
        env: Env,
        caller: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
    ) {
        storage::renew_governance_instance(&env);
        let tolerance = validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        );
        controller_client(&env).set_oracle_tolerance(&asset, &tolerance);
    }
}
