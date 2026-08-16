//! Resolves each `AdminOperation` variant to the concrete cross-contract call
//! (target address, function symbol, encoded arguments, and timelock delay
//! tier) that the timelock schedules and later executes, and applies the
//! subset of operations that target the governance contract itself.

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
    AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, DeployPositionNftArgs,
    EditToleranceArgs, RemoveAssetFromSpokeArgs, RoleArgs, SpokeAssetArgs,
    SpokeLiquidationCurveArgs, TransferOwnershipArgs, UpgradePoolParamsArgs,
};

/// Validates risk bounds, liquidation fees, and supply/borrow caps for a
/// spoke-asset add or edit operation. Panics if any of the checks fail.
fn validate_spoke_asset(env: &Env, args: &SpokeAssetArgs) {
    validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    validate_liquidation_fees(env, args.liquidation_fees);
    validate::asset::validate_spoke_cap_args(env, args.supply_cap, args.borrow_cap);
}

/// Returns a copy of `oracle` with `asset_decimals` set from the on-chain
/// token contract for a `PriceKey::Token` key, or `0` for a `PriceKey::Ref`
/// key.
pub(crate) fn resolve_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) -> AssetOracle {
    let mut resolved = oracle.clone();
    resolved.asset_decimals = match key {
        PriceKey::Token(asset) => validate::asset::validate_and_fetch_token_decimals(env, asset),
        PriceKey::Ref(_) => 0,
    };
    resolved
}

/// The concrete cross-contract call a resolved `AdminOperation` maps to:
/// which contract to invoke, which function, with which encoded arguments,
/// and under which timelock delay tier.
pub(crate) struct ResolvedOperation {
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub delay_tier: DelayTier,
}

/// Builds a `ResolvedOperation` targeting the controller contract with the
/// `Standard` delay tier.
fn controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_controller(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Standard,
    }
}

/// Builds a `ResolvedOperation` targeting the controller contract with the
/// `Sensitive` delay tier.
fn sensitive_controller_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_controller(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Sensitive,
    }
}

/// Builds a `ResolvedOperation` targeting the price aggregator contract with
/// the `Standard` delay tier.
fn price_aggregator_operation(env: &Env, function: &str, args: Vec<Val>) -> ResolvedOperation {
    ResolvedOperation {
        target: storage::get_price_aggregator(env),
        function: Symbol::new(env, function),
        args,
        delay_tier: DelayTier::Standard,
    }
}

/// Builds a `ResolvedOperation` targeting the governance contract itself
/// (`env.current_contract_address()`) with the given delay tier.
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

/// Validates the arguments carried by `op` and maps it to the
/// `ResolvedOperation` (target, function, encoded arguments, delay tier) the
/// timelock queues for later execution. Panics if the operation's arguments
/// fail validation.
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
            sensitive_controller_operation(
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
        AdminOperation::ApproveBlendPool(pool) => {
            validate::require_contract_address(env, pool, GenericError::NotSmartContract);
            controller_operation(
                env,
                "approve_blend_pool",
                vec![env, pool.clone().into_val(env)],
            )
        }
        AdminOperation::RevokeBlendPool(pool) => {
            validate::require_contract_address(env, pool, GenericError::NotSmartContract);
            controller_operation(
                env,
                "revoke_blend_pool",
                vec![env, pool.clone().into_val(env)],
            )
        }
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
        AdminOperation::DeployPositionNft(args) => {
            validate::require_nonzero_wasm_hash(env, &args.wasm_hash);
            controller_operation(
                env,
                "deploy_position_nft",
                vec![
                    env,
                    args.wasm_hash.clone().into_val(env),
                    args.uri.clone().into_val(env),
                    args.name.clone().into_val(env),
                    args.symbol.clone().into_val(env),
                ],
            )
        }
        AdminOperation::UpgradePool(hash) => {
            validate::require_nonzero_wasm_hash(env, hash);
            sensitive_controller_operation(
                env,
                "upgrade_pool",
                vec![env, hash.clone().into_val(env)],
            )
        }
        AdminOperation::SetPositionManager(manager, is_active) => sensitive_controller_operation(
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
            let oracle = resolve_oracle(env, &args.key, &args.oracle);
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

/// Applies the effect of an `AdminOperation` whose resolved target is the
/// governance contract itself, in place of an `invoke_contract` call.
/// `SetPriceAggregator` also stores the new address locally and forwards the
/// update to the controller. Panics with `GenericError::InternalError` for
/// any operation whose resolved target is a different contract.
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
        | AdminOperation::DeployPositionNft(_)
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
