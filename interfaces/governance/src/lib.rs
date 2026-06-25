#![no_std]
#![allow(clippy::too_many_arguments)]

//! Published ABI for the governance contract.
//!
//! Mirrors the governance contract's public entrypoint surface for typed
//! cross-contract and off-chain callers. Client-only: `#[contractclient]`
//! generates `GovernanceClient`; the governance contract does NOT formally
//! `impl` this trait — its entrypoints match by ABI name.

use common::types::{InterestRateModel, MarketParamsRaw};
use controller_interface::types::{
    AssetConfigRaw, MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
    PositionLimits,
};
use soroban_sdk::{contractclient, contracttype, Address, BytesN, Env, Symbol, Val, Vec};
pub use stellar_governance::timelock::OperationState;

pub use stellar_governance::timelock::OperationState as GovernanceOperationState;

#[contracttype]
#[derive(Clone, Debug)]
pub struct EModeAssetArgs {
    pub asset: Address,
    pub category_id: u32,
    pub can_collateral: bool,
    pub can_borrow: bool,
    pub ltv: u32,
    pub threshold: u32,
    pub bonus: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolCapsArgs {
    pub asset: Address,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RemoveAssetFromEModeArgs {
    pub asset: Address,
    pub category_id: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreatePoolArgs {
    pub asset: Address,
    pub params: MarketParamsRaw,
    pub config: AssetConfigRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct UpgradePoolParamsArgs {
    pub asset: Address,
    pub params: InterestRateModel,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TransferOwnershipArgs {
    pub new_owner: Address,
    pub live_until_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ConfigureOracleArgs {
    pub asset: Address,
    pub cfg: MarketOracleConfigInput,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EditToleranceArgs {
    pub asset: Address,
    pub first_tolerance: u32,
    pub last_tolerance: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RoleArgs {
    pub account: Address,
    pub role: Symbol,
}

#[contracttype]
#[derive(Clone, Debug)]
pub enum AdminOperation {
    // Controller target
    SetAggregator(Address),
    SetAccumulator(Address),
    SetLiquidityPoolTemplate(BytesN<32>),
    EditAssetConfig(Address, AssetConfigRaw),
    SetPositionLimits(PositionLimits),
    SetMinBorrowCollateralUsd(i128),
    AddEModeCategory,
    RemoveEModeCategory(u32),
    AddAssetToEModeCategory(EModeAssetArgs),
    EditAssetInEModeCategory(EModeAssetArgs),
    UpdatePoolCaps(PoolCapsArgs),
    RemoveAssetFromEMode(RemoveAssetFromEModeArgs),
    ApproveToken(Address),
    RevokeToken(Address),
    ApproveBlendPool(Address),
    RevokeBlendPool(Address),
    CreateLiquidityPool(CreatePoolArgs),
    UpgradeLiquidityPoolParams(UpgradePoolParamsArgs),
    DeployPool,
    UpgradePool(BytesN<32>),
    DisableTokenOracle(Address),
    UpgradeController(BytesN<32>),
    MigrateController(u32),
    TransferCtrlOwnership(TransferOwnershipArgs),
    ConfigureMarketOracle(ConfigureOracleArgs),
    EditOracleTolerance(EditToleranceArgs),

    // Governance target (Self)
    UpgradeGov(BytesN<32>),
    UpdateGovDelay(u32),
    GrantGovRole(RoleArgs),
    RevokeGovRole(RoleArgs),
    TransferGovOwnership(TransferOwnershipArgs),
}

#[contractclient(name = "GovernanceClient")]
/// Governance contract interface: controller deployment, the timelock
/// lifecycle, the typed controller-admin proposers, and governance-self admin.
pub trait GovernanceInterface {
    // --- deploy.rs: one-time controller deployment and lookup ---

    /// One-time controller deployment; returns the deployed controller address.
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address;

    /// Returns the deployed controller address.
    fn controller(env: Env) -> Address;

    // --- timelock.rs: generic lifecycle, delay setter, and read views ---

    /// Executes a ready operation, invoking the controller; `Some(executor)`
    /// gates on EXECUTOR, `None` leaves execution open.
    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val;

    /// Cancels a pending operation; the caller must hold CANCELLER.
    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>);

    /// Minimum timelock delay in ledgers.
    fn get_min_delay(env: Env) -> u32;

    /// Current lifecycle state of an operation.
    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState;

    /// Ledger at which an operation becomes ready (`0` unset, `1` done).
    fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32;

    /// Deterministic operation id for the given fields.
    fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Resolves a market oracle input to the `MarketOracleConfig` the matching
    /// proposer schedules; read-only.
    fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig;

    /// Resolves tolerance BPS inputs to the `OraclePriceFluctuation` the
    /// matching proposer schedules; read-only.
    fn resolve_oracle_tolerance(
        env: Env,
        first_tolerance: u32,
        last_tolerance: u32,
    ) -> OraclePriceFluctuation;

    // --- forward.rs: generic proposer ---

    /// Schedules an administrative operation.
    fn propose(
        env: Env,
        proposer: Address,
        op: AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Emergency brake: halts the controller immediately, owner-gated.
    fn pause(env: Env);

    /// Resumes the controller, owner-gated and immediate.
    fn unpause(env: Env);

    // --- access.rs / self_timelock.rs: governance-self administration ---

    /// Executes a scheduled self-operation.
    fn execute_self(
        env: Env,
        executor: Option<Address>,
        op: AdminOperation,
        salt: BytesN<32>,
    );

    /// Accepts a pending ownership transfer of the governance contract.
    fn accept_ownership(env: Env);

    /// Returns whether `account` holds `role` on the governance contract.
    fn has_role(env: Env, account: Address, role: Symbol) -> bool;
}
