//! `AdminOperation` resolution and self-operation application.
//!
//! `resolve_op` validates each operation's inputs and lowers it to a
//! `(target, function, args, delay-tier)` timelock call; `apply_self_op`
//! executes the governance-self variants inline once their timelock matures.

use common::errors::{CollateralError, GenericError, OracleError};

use soroban_sdk::{
    assert_with_error, panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec,
};

use crate::access;
use crate::timelock::{apply_update_delay, validate_delay_update, DelayTier};
use crate::{storage, validate};

pub use governance_interface::{
    AdminOperation, ConfigureOracleArgs, CreatePoolArgs, EditToleranceArgs,
    RemoveAssetFromSpokeArgs, RoleArgs, SpokeAssetArgs, SpokeLiquidationCurveArgs,
    TransferOwnershipArgs, UpgradePoolParamsArgs,
};

fn validate_spoke_asset(env: &Env, args: &SpokeAssetArgs) {
    validate::asset::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    validate::asset::validate_liquidation_fees(env, args.liquidation_fees);
    validate::asset::validate_spoke_cap_args(env, args.supply_cap, args.borrow_cap);
}

type ResolvedOperation = (Address, Symbol, Vec<Val>, DelayTier);

fn controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    (
        storage::get_controller(env),
        Symbol::new(env, function),
        args,
        DelayTier::Standard,
    )
}

fn sensitive_controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    (
        storage::get_controller(env),
        Symbol::new(env, function),
        args,
        DelayTier::Sensitive,
    )
}

/// Oracle config ops target the price-aggregator (the oracle authority), not
/// the controller.
fn price_aggregator_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    (
        storage::get_price_aggregator(env),
        Symbol::new(env, function),
        args,
        DelayTier::Standard,
    )
}

pub(crate) fn resolve_op(env: &Env, op: &AdminOperation) -> ResolvedOperation {
    let gov_addr = env.current_contract_address();

    match op {
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
            access::require_known_governance_role(env, &args.role);
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
            access::require_known_governance_role(env, &args.role);
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

        AdminOperation::SetSwapAggregator(addr) => {
            validate::require_contract_address(env, addr, OracleError::InvalidAggregator);
            controller_operation(
                env,
                "set_swap_aggregator",
                vec![env, addr.clone().into_val(env)],
            )
        }
        AdminOperation::SetPriceAggregator(addr) => {
            validate::require_contract_address(env, addr, OracleError::InvalidAggregator);
            controller_operation(
                env,
                "set_price_aggregator",
                vec![env, addr.clone().into_val(env)],
            )
        }
        AdminOperation::SetAccumulator(addr) => controller_operation(
            env,
            "set_accumulator",
            vec![env, addr.clone().into_val(env)],
        ),
        AdminOperation::SetLiquidityPoolTemplate(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            controller_operation(
                env,
                "set_liquidity_pool_template",
                vec![env, hash.clone().into_val(env)],
            )
        }
        AdminOperation::SetPositionLimits(limits) => {
            validate::asset::validate_position_limits(env, limits);
            controller_operation(
                env,
                "set_position_limits",
                vec![env, limits.clone().into_val(env)],
            )
        }
        AdminOperation::SetMinBorrowCollateralUsd(floor_wad) => {
            assert_with_error!(env, *floor_wad >= 0, CollateralError::InvalidBorrowParams);
            controller_operation(
                env,
                "set_min_borrow_collateral_usd",
                vec![env, floor_wad.into_val(env)],
            )
        }
        AdminOperation::CreateHub => controller_operation(env, "create_hub", vec![env]),
        AdminOperation::AddSpoke => controller_operation(env, "add_spoke", vec![env]),
        AdminOperation::RemoveSpoke(id) => {
            controller_operation(env, "remove_spoke", vec![env, id.into_val(env)])
        }
        AdminOperation::AddAssetToSpoke(args) => {
            validate_spoke_asset(env, args);
            controller_operation(
                env,
                "add_asset_to_spoke",
                vec![env, args.clone().into_val(env)],
            )
        }
        AdminOperation::EditAssetInSpoke(args) => {
            validate_spoke_asset(env, args);
            controller_operation(
                env,
                "edit_asset_in_spoke",
                vec![env, args.clone().into_val(env)],
            )
        }
        AdminOperation::RemoveAssetFromSpoke(args) => controller_operation(
            env,
            "remove_asset_from_spoke",
            vec![
                env,
                args.hub_asset.clone().into_val(env),
                args.spoke_id.into_val(env),
            ],
        ),
        AdminOperation::ApproveBlendPool(pool) => controller_operation(
            env,
            "approve_blend_pool",
            vec![env, pool.clone().into_val(env)],
        ),
        AdminOperation::RevokeBlendPool(pool) => controller_operation(
            env,
            "revoke_blend_pool",
            vec![env, pool.clone().into_val(env)],
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
            controller_operation(
                env,
                "create_liquidity_pool",
                vec![
                    env,
                    args.hub_id.into_val(env),
                    args.asset.clone().into_val(env),
                    args.params.clone().into_val(env),
                ],
            )
        }
        AdminOperation::UpgradeLiquidityPoolParams(args) => {
            args.params.verify(env);
            controller_operation(
                env,
                "upgrade_liquidity_pool_params",
                vec![
                    env,
                    args.hub_asset.clone().into_val(env),
                    args.params.clone().into_val(env),
                ],
            )
        }
        AdminOperation::DeployPool => controller_operation(env, "deploy_pool", vec![env]),
        AdminOperation::UpgradePool(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            sensitive_controller_operation(
                env,
                "upgrade_pool",
                vec![env, hash.clone().into_val(env)],
            )
        }
        AdminOperation::SetPositionManager(manager, is_active) => controller_operation(
            env,
            "set_position_manager",
            vec![env, manager.clone().into_val(env), is_active.into_val(env)],
        ),
        AdminOperation::UpgradeController(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            sensitive_controller_operation(env, "upgrade", vec![env, hash.clone().into_val(env)])
        }
        AdminOperation::MigrateController(version) => {
            controller_operation(env, "migrate", vec![env, version.into_val(env)])
        }
        AdminOperation::TransferCtrlOwnership(args) => {
            validate::require_contract_address(
                env,
                &args.new_owner,
                GenericError::NotSmartContract,
            );
            sensitive_controller_operation(
                env,
                "transfer_ownership",
                vec![
                    env,
                    args.new_owner.clone().into_val(env),
                    args.live_until_ledger.into_val(env),
                ],
            )
        }
        AdminOperation::ConfigureMarketOracle(args) => {
            let tolerance =
                validate::tolerance::validate_and_calculate_tolerances(env, args.cfg.tolerance_bps);
            let resolved_config = validate::oracle_probe::validate_market_oracle_sources(
                env,
                &args.hub_asset.asset,
                &args.cfg,
                tolerance,
            );
            price_aggregator_operation(
                env,
                "set_market_oracle_config",
                vec![
                    env,
                    args.hub_asset.asset.clone().into_val(env),
                    resolved_config.into_val(env),
                ],
            )
        }
        AdminOperation::EditOracleTolerance(args) => {
            let tolerance =
                validate::tolerance::validate_and_calculate_tolerances(env, args.tolerance);
            price_aggregator_operation(
                env,
                "set_oracle_tolerance",
                vec![
                    env,
                    args.asset.clone().into_val(env),
                    tolerance.into_val(env),
                ],
            )
        }
        AdminOperation::Unpause => controller_operation(env, "unpause", vec![env]),
        AdminOperation::SetSpokeLiquidationCurve(args) => {
            validate::spoke::validate_liquidation_curve(
                env,
                args.target_hf_wad,
                args.hf_for_max_bonus_wad,
                args.liquidation_bonus_factor_bps,
            );
            controller_operation(
                env,
                "set_spoke_liquidation_curve",
                vec![
                    env,
                    args.spoke_id.into_val(env),
                    args.target_hf_wad.into_val(env),
                    args.hf_for_max_bonus_wad.into_val(env),
                    args.liquidation_bonus_factor_bps.into_val(env),
                ],
            )
        }
    }
}

pub(crate) fn apply_self_op(env: &Env, op: &AdminOperation) {
    match op {
        AdminOperation::UpgradeGov(hash) => access::apply_upgrade(env, hash),
        AdminOperation::UpdateGovDelay(new_delay) => apply_update_delay(env, *new_delay),
        AdminOperation::GrantGovRole(args) => {
            access::apply_grant_role(env, &args.account, &args.role)
        }
        AdminOperation::RevokeGovRole(args) => {
            access::apply_revoke_role(env, &args.account, &args.role)
        }
        AdminOperation::TransferGovOwnership(args) => {
            access::apply_transfer_ownership(env, &args.new_owner, args.live_until_ledger)
        }
        // Only self-targeted operations reach `execute_self`.
        _ => panic_with_error!(env, GenericError::InternalError),
    }
}

#[cfg(test)]
#[path = "../tests/op.rs"]
mod xdr_parity;
