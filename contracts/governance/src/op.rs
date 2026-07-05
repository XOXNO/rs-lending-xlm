//! `AdminOperation` resolution and self-operation application.
//!
//! `resolve_op` validates each operation's inputs and lowers it to a
//! `(target, function, args, delay-tier)` timelock call; `apply_self_op`
//! executes the governance-self variants inline once their timelock matures.

use common::errors::{CollateralError, GenericError, OracleError};
use soroban_sdk::{
    assert_with_error, panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec,
};

use crate::timelock::{validate_delay_update, DelayTier};
use crate::{storage, validate};

pub use governance_interface::{
    AdminOperation, ConfigureOracleArgs, CreatePoolArgs, EditToleranceArgs,
    RemoveAssetFromSpokeArgs, RoleArgs, SpokeAssetArgs, TransferOwnershipArgs,
    UpgradePoolParamsArgs,
};

pub(crate) fn resolve_op(env: &Env, op: &AdminOperation) -> (Address, Symbol, Vec<Val>, DelayTier) {
    let gov_addr = env.current_contract_address();

    match op {
        // --- Governance target (Self) ---
        AdminOperation::UpgradeGov(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            (
                gov_addr,
                Symbol::new(env, "upgrade"),
                vec![env, hash.clone().into_val(env)],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::UpdateGovDelay(new_delay) => {
            validate_delay_update(env, *new_delay);
            (
                gov_addr,
                Symbol::new(env, "update_delay"),
                vec![env, new_delay.into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::GrantGovRole(args) => {
            crate::access::require_known_governance_role(env, &args.role);
            (
                gov_addr,
                Symbol::new(env, "grant_role"),
                vec![
                    env,
                    args.account.clone().into_val(env),
                    args.role.clone().into_val(env),
                ],
                DelayTier::Standard,
            )
        }
        AdminOperation::RevokeGovRole(args) => {
            crate::access::require_known_governance_role(env, &args.role);
            (
                gov_addr,
                Symbol::new(env, "revoke_role"),
                vec![
                    env,
                    args.account.clone().into_val(env),
                    args.role.clone().into_val(env),
                ],
                DelayTier::Standard,
            )
        }
        AdminOperation::TransferGovOwnership(args) => (
            gov_addr,
            Symbol::new(env, "transfer_ownership"),
            vec![
                env,
                args.new_owner.clone().into_val(env),
                args.live_until_ledger.into_val(env),
            ],
            DelayTier::Sensitive,
        ),

        // --- Controller target ---
        AdminOperation::SetAggregator(addr) => {
            validate::require_contract_address(env, addr, OracleError::InvalidAggregator);
            (
                storage::get_controller(env),
                Symbol::new(env, "set_aggregator"),
                vec![env, addr.clone().into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::SetAccumulator(addr) => (
            storage::get_controller(env),
            Symbol::new(env, "set_accumulator"),
            vec![env, addr.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::SetLiquidityPoolTemplate(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            (
                storage::get_controller(env),
                Symbol::new(env, "set_liquidity_pool_template"),
                vec![env, hash.clone().into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::SetPositionLimits(limits) => {
            validate::asset::validate_position_limits(env, limits);
            (
                storage::get_controller(env),
                Symbol::new(env, "set_position_limits"),
                vec![env, limits.clone().into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::SetMinBorrowCollateralUsd(floor_wad) => {
            assert_with_error!(env, *floor_wad >= 0, CollateralError::InvalidBorrowParams);
            (
                storage::get_controller(env),
                Symbol::new(env, "set_min_borrow_collateral_usd"),
                vec![env, floor_wad.into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::CreateHub => (
            storage::get_controller(env),
            Symbol::new(env, "create_hub"),
            vec![env],
            DelayTier::Standard,
        ),
        AdminOperation::AddSpoke => (
            storage::get_controller(env),
            Symbol::new(env, "add_spoke"),
            vec![env],
            DelayTier::Standard,
        ),
        AdminOperation::RemoveSpoke(id) => (
            storage::get_controller(env),
            Symbol::new(env, "remove_spoke"),
            vec![env, id.into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::AddAssetToSpoke(args) => {
            validate::asset::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
            validate::asset::validate_liquidation_fees(env, args.liquidation_fees);
            validate::asset::validate_spoke_cap_args(env, args.supply_cap, args.borrow_cap);
            (
                storage::get_controller(env),
                Symbol::new(env, "add_asset_to_spoke"),
                vec![env, args.clone().into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::EditAssetInSpoke(args) => {
            validate::asset::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
            validate::asset::validate_liquidation_fees(env, args.liquidation_fees);
            validate::asset::validate_spoke_cap_args(env, args.supply_cap, args.borrow_cap);
            (
                storage::get_controller(env),
                Symbol::new(env, "edit_asset_in_spoke"),
                vec![env, args.clone().into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::RemoveAssetFromSpoke(args) => (
            storage::get_controller(env),
            Symbol::new(env, "remove_asset_from_spoke"),
            vec![
                env,
                args.hub_asset.clone().into_val(env),
                args.spoke_id.into_val(env),
            ],
            DelayTier::Standard,
        ),
        AdminOperation::ApproveToken(token) => (
            storage::get_controller(env),
            Symbol::new(env, "approve_token"),
            vec![env, token.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::RevokeToken(token) => (
            storage::get_controller(env),
            Symbol::new(env, "revoke_token"),
            vec![env, token.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::ApproveBlendPool(pool) => (
            storage::get_controller(env),
            Symbol::new(env, "approve_blend_pool"),
            vec![env, pool.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::RevokeBlendPool(pool) => (
            storage::get_controller(env),
            Symbol::new(env, "revoke_blend_pool"),
            vec![env, pool.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::CreateLiquidityPool(args) => {
            let token_decimals =
                validate::asset::validate_and_fetch_token_decimals(env, &args.asset);
            validate::asset::validate_market_creation(
                env,
                &args.asset,
                &args.params,
                token_decimals,
            );
            (
                storage::get_controller(env),
                Symbol::new(env, "create_liquidity_pool"),
                vec![
                    env,
                    args.hub_id.into_val(env),
                    args.asset.clone().into_val(env),
                    args.params.clone().into_val(env),
                ],
                DelayTier::Standard,
            )
        }
        AdminOperation::UpgradeLiquidityPoolParams(args) => {
            args.params.verify(env);
            (
                storage::get_controller(env),
                Symbol::new(env, "upgrade_liquidity_pool_params"),
                vec![
                    env,
                    args.hub_asset.clone().into_val(env),
                    args.params.clone().into_val(env),
                ],
                DelayTier::Standard,
            )
        }
        AdminOperation::DeployPool => (
            storage::get_controller(env),
            Symbol::new(env, "deploy_pool"),
            vec![env],
            DelayTier::Standard,
        ),
        AdminOperation::UpgradePool(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            (
                storage::get_controller(env),
                Symbol::new(env, "upgrade_pool"),
                vec![env, hash.clone().into_val(env)],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::DisableTokenOracle(asset) => (
            storage::get_controller(env),
            Symbol::new(env, "disable_token_oracle"),
            vec![env, asset.clone().into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::SetPositionManager(manager, is_active) => (
            storage::get_controller(env),
            Symbol::new(env, "set_position_manager"),
            vec![env, manager.clone().into_val(env), is_active.into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::UpgradeController(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            (
                storage::get_controller(env),
                Symbol::new(env, "upgrade"),
                vec![env, hash.clone().into_val(env)],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::MigrateController(version) => (
            storage::get_controller(env),
            Symbol::new(env, "migrate"),
            vec![env, version.into_val(env)],
            DelayTier::Standard,
        ),
        AdminOperation::TransferCtrlOwnership(args) => {
            validate::require_contract_address(
                env,
                &args.new_owner,
                GenericError::NotSmartContract,
            );
            (
                storage::get_controller(env),
                Symbol::new(env, "transfer_ownership"),
                vec![
                    env,
                    args.new_owner.clone().into_val(env),
                    args.live_until_ledger.into_val(env),
                ],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::ConfigureMarketOracle(args) => {
            let tolerance =
                validate::tolerance::validate_and_calculate_tolerances(env, args.cfg.tolerance_bps);
            let controller = storage::get_controller(env);
            let resolved_config = validate::oracle_probe::validate_market_oracle_sources(
                env,
                &args.hub_asset.asset,
                &args.cfg,
                tolerance,
            );
            (
                controller,
                Symbol::new(env, "set_market_oracle_config"),
                vec![
                    env,
                    args.hub_asset.clone().into_val(env),
                    resolved_config.into_val(env),
                ],
                DelayTier::Standard,
            )
        }
        AdminOperation::EditOracleTolerance(args) => {
            let tolerance =
                validate::tolerance::validate_and_calculate_tolerances(env, args.tolerance);
            (
                storage::get_controller(env),
                Symbol::new(env, "set_oracle_tolerance"),
                vec![
                    env,
                    args.asset.clone().into_val(env),
                    tolerance.into_val(env),
                ],
                DelayTier::Standard,
            )
        }
    }
}

pub(crate) fn apply_self_op(env: &Env, op: &AdminOperation) {
    match op {
        AdminOperation::UpgradeGov(hash) => {
            crate::access::apply_upgrade(env, hash);
        }
        AdminOperation::UpdateGovDelay(new_delay) => {
            crate::timelock::apply_update_delay(env, *new_delay);
        }
        AdminOperation::GrantGovRole(args) => {
            crate::access::apply_grant_role(env, &args.account, &args.role);
        }
        AdminOperation::RevokeGovRole(args) => {
            crate::access::apply_revoke_role(env, &args.account, &args.role);
        }
        AdminOperation::TransferGovOwnership(args) => {
            crate::access::apply_transfer_ownership(env, &args.new_owner, args.live_until_ledger);
        }
        // Only self-targeted operations reach `execute_self`.
        _ => panic_with_error!(env, GenericError::InternalError),
    }
}

#[cfg(test)]
#[path = "../tests/op.rs"]
mod xdr_parity;
