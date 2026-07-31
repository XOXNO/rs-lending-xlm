use common::errors::{CollateralError, GenericError, OracleError};
use common::types::{AssetOracle, PriceKey};
use common::validation::{
    validate_liquidation_curve, validate_liquidation_fees, validate_risk_bounds,
};

use soroban_sdk::{
    assert_with_error, panic_with_error, vec, Address, Env, IntoVal, Symbol, Val, Vec,
};

use crate::access;
use crate::timelock::{apply_update_delay, validate_delay_update, DelayTier};
use crate::{storage, validate};

pub use governance_interface::{
    AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, EditToleranceArgs,
    RemoveAssetFromSpokeArgs, RoleArgs, SpokeAssetArgs, SpokeLiquidationCurveArgs,
    TransferOwnershipArgs, UpgradePoolParamsArgs,
};

fn validate_spoke_asset(env: &Env, args: &SpokeAssetArgs) {
    validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    validate_liquidation_fees(env, args.liquidation_fees);
    validate::asset::validate_spoke_cap_args(env, args.supply_cap, args.borrow_cap);
}

pub(crate) fn resolve_asset_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) -> AssetOracle {
    let mut resolved = oracle.clone();
    resolved.asset_decimals = match key {
        PriceKey::Token(asset) => validate::asset::validate_and_fetch_token_decimals(env, asset),
        PriceKey::Ref(_) => 0,
    };
    resolved
}

pub(crate) struct ResolvedOperation {
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub delay_tier: DelayTier,
}

fn controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_controller(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Standard,
    }
}

fn sensitive_controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_controller(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Sensitive,
    }
}

fn price_aggregator_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_price_aggregator(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Standard,
    }
}

fn self_operation(
    env: &Env,
    function: &str,
    args: Vec<Val>,
    delay_tier: DelayTier,
) -> ResolvedOperation {
    ResolvedOperation {
        target: env.current_contract_address(),
        function: Symbol::new(env, function),
        args,
        delay_tier,
    }
}

pub(crate) fn resolve_op(env: &Env, op: &AdminOperation) -> ResolvedOperation {
    match op {
        AdminOperation::UpgradeGov(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            self_operation(
                env,
                "upgrade",
                vec![env, hash.clone().into_val(env)],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::UpdateGovDelay(new_delay) => {
            validate_delay_update(env, *new_delay);
            self_operation(
                env,
                "update_delay",
                vec![env, new_delay.into_val(env)],
                DelayTier::Standard,
            )
        }
        AdminOperation::GrantGovRole(args) => {
            access::require_known_governance_role(env, &args.role);
            self_operation(
                env,
                "grant_role",
                vec![
                    env,
                    args.account.clone().into_val(env),
                    args.role.clone().into_val(env),
                ],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::RevokeGovRole(args) => {
            access::require_known_governance_role(env, &args.role);
            self_operation(
                env,
                "revoke_role",
                vec![
                    env,
                    args.account.clone().into_val(env),
                    args.role.clone().into_val(env),
                ],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::TransferGovOwnership(args) => self_operation(
            env,
            "transfer_ownership",
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
            self_operation(
                env,
                "set_price_aggregator",
                vec![env, addr.clone().into_val(env)],
                DelayTier::Sensitive,
            )
        }
        AdminOperation::SetAccumulator(addr) => controller_operation(
            env,
            "set_accumulator",
            vec![env, addr.clone().into_val(env)],
        ),
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
        AdminOperation::DeployPool(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            controller_operation(env, "deploy_pool", vec![env, hash.clone().into_val(env)])
        }
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
        AdminOperation::ConfigureAssetOracle(args) => {
            let oracle = resolve_asset_oracle(env, &args.key, &args.oracle);
            price_aggregator_operation(
                env,
                "set_oracle",
                vec![env, args.key.clone().into_val(env), oracle.into_val(env)],
            )
        }
        AdminOperation::EditOracleTolerance(args) => {
            let tolerance =
                validate::tolerance::validate_and_calculate_tolerances(env, args.tolerance);
            price_aggregator_operation(
                env,
                "set_tolerance",
                vec![env, args.key.clone().into_val(env), tolerance.into_val(env)],
            )
        }
        AdminOperation::Unpause => controller_operation(env, "unpause", vec![env]),
        AdminOperation::ForceSocializeBadDebt(account_id) => sensitive_controller_operation(
            env,
            "force_socialize_bad_debt",
            vec![env, account_id.into_val(env)],
        ),
        AdminOperation::SetSpokeLiquidationCurve(args) => {
            validate_liquidation_curve(
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
        AdminOperation::SetPriceAggregator(addr) => {
            validate::require_contract_address(env, addr, OracleError::InvalidAggregator);
            storage::set_price_aggregator(env, addr);
            env.invoke_contract::<Val>(
                &storage::get_controller(env),
                &Symbol::new(env, "set_price_aggregator"),
                vec![env, addr.clone().into_val(env)],
            );
        }
        AdminOperation::SetSwapAggregator(_)
        | AdminOperation::SetAccumulator(_)
        | AdminOperation::SetPositionLimits(_)
        | AdminOperation::SetMinBorrowCollateralUsd(_)
        | AdminOperation::CreateHub
        | AdminOperation::AddSpoke
        | AdminOperation::RemoveSpoke(_)
        | AdminOperation::AddAssetToSpoke(_)
        | AdminOperation::EditAssetInSpoke(_)
        | AdminOperation::RemoveAssetFromSpoke(_)
        | AdminOperation::ApproveBlendPool(_)
        | AdminOperation::RevokeBlendPool(_)
        | AdminOperation::CreateLiquidityPool(_)
        | AdminOperation::UpgradeLiquidityPoolParams(_)
        | AdminOperation::DeployPool(_)
        | AdminOperation::UpgradePool(_)
        | AdminOperation::SetPositionManager(_, _)
        | AdminOperation::UpgradeController(_)
        | AdminOperation::MigrateController(_)
        | AdminOperation::TransferCtrlOwnership(_)
        | AdminOperation::ConfigureAssetOracle(_)
        | AdminOperation::EditOracleTolerance(_)
        | AdminOperation::SetSpokeLiquidationCurve(_)
        | AdminOperation::ForceSocializeBadDebt(_)
        | AdminOperation::Unpause => panic_with_error!(env, GenericError::InternalError),
    }
}

#[cfg(test)]
#[path = "../tests/op.rs"]
mod xdr_parity;
