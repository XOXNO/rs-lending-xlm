#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only governance ABI (`GovernanceClient`). Matches deploy entrypoints by name.

use common::types::{
    HubAssetKey, MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
    PositionLimits,
};
use common::types::{InterestRateModel, MarketParamsRaw};
use soroban_sdk::{contractclient, contracttype, Address, BytesN, Env, Symbol, Val, Vec};
pub use stellar_governance::timelock::OperationState;

pub use stellar_governance::timelock::OperationState as GovernanceOperationState;

/// Spoke asset input forwarded to controller spoke-asset admin entrypoints without mutation.
pub use common::types::SpokeAssetArgs;

#[contracttype]
#[derive(Clone, Debug)]
pub struct RemoveAssetFromSpokeArgs {
    pub hub_asset: HubAssetKey,
    pub spoke_id: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreatePoolArgs {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct UpgradePoolParamsArgs {
    pub hub_asset: HubAssetKey,
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
    pub hub_asset: HubAssetKey,
    pub cfg: MarketOracleConfigInput,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EditToleranceArgs {
    pub asset: Address,
    pub tolerance: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SpokeLiquidationCurveArgs {
    pub spoke_id: u32,
    pub target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RoleArgs {
    pub account: Address,
    pub role: Symbol,
}

#[contracttype]
#[derive(Clone, Debug)]
// `#[contracttype]` enums cannot box variants (Soroban has no `Box` codec);
// `CreateLiquidityPool` embeds large `MarketParamsRaw`. Mirrors allow on
// `MarketOracleConfigOption`.
#[allow(clippy::large_enum_variant)]
pub enum AdminOperation {
    // Controller target
    SetAggregator(Address),
    SetAccumulator(Address),
    SetLiquidityPoolTemplate(BytesN<32>),
    SetPositionLimits(PositionLimits),
    SetMinBorrowCollateralUsd(i128),
    CreateHub,
    AddSpoke,
    RemoveSpoke(u32),
    AddAssetToSpoke(SpokeAssetArgs),
    EditAssetInSpoke(SpokeAssetArgs),
    RemoveAssetFromSpoke(RemoveAssetFromSpokeArgs),
    ApproveToken(Address),
    RevokeToken(Address),
    ApproveBlendPool(Address),
    RevokeBlendPool(Address),
    CreateLiquidityPool(CreatePoolArgs),
    UpgradeLiquidityPoolParams(UpgradePoolParamsArgs),
    DeployPool,
    UpgradePool(BytesN<32>),
    SetPositionManager(Address, bool),
    UpgradeController(BytesN<32>),
    MigrateController(u32),
    TransferCtrlOwnership(TransferOwnershipArgs),
    ConfigureMarketOracle(ConfigureOracleArgs),
    EditOracleTolerance(EditToleranceArgs),
    SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs),
    /// Risk-loosening: rides the timelock. Counterpart to GUARDIAN-immediate `pause`.
    Unpause,

    // Governance target (Self)
    UpgradeGov(BytesN<32>),
    UpdateGovDelay(u32),
    GrantGovRole(RoleArgs),
    RevokeGovRole(RoleArgs),
    TransferGovOwnership(TransferOwnershipArgs),
}

#[contractclient(name = "GovernanceClient")]
/// Controller deployment, timelock lifecycle, typed admin proposers, self-admin.
pub trait GovernanceInterface {
    /// One-time controller deploy; returns address.
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address;

    fn controller(env: Env) -> Address;

    /// Executes a ready operation on the controller. `Some(executor)` gates on
    /// EXECUTOR; `None` leaves execution open.
    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val;

    /// Cancels a pending operation; caller must hold CANCELLER.
    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>);

    /// Minimum timelock delay in ledgers.
    fn get_min_delay(env: Env) -> u32;

    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState;

    /// Ledger when operation becomes ready (`0` unset, `1` done).
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

    /// Read-only: resolves oracle input to the `MarketOracleConfig` proposers schedule.
    fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig;

    /// Read-only: resolves tolerance BPS to the `OraclePriceFluctuation` proposers schedule.
    fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OraclePriceFluctuation;

    fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>;

    /// Emergency brake: halts controller immediately; GUARDIAN-gated.
    /// Resume is timelocked `AdminOperation::Unpause`.
    fn pause(env: Env, caller: Address);

    /// Sets spoke listing paused/frozen immediately; GUARDIAN-gated.
    /// Tighten-only: clearing a set flag rides the timelocked edit path.
    fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    );

    /// Moves oracle sanity band immediately; ORACLE-gated.
    /// New band must contain the current live price.
    fn set_oracle_sanity_bounds(
        env: Env,
        caller: Address,
        asset: Address,
        min_wad: i128,
        max_wad: i128,
    );

    /// Immediate hub create; GUARDIAN-gated. Returns new hub id.
    fn create_hub(env: Env, caller: Address) -> u32;

    /// Immediate spoke create; GUARDIAN-gated. Returns new spoke id.
    fn add_spoke(env: Env, caller: Address) -> u32;

    /// Immediate GUARDIAN/ORACLE revoke; owner-gated emergency de-auth.
    /// Grants and PROPOSER/EXECUTOR/CANCELLER revokes stay timelocked.
    fn revoke_role_immediate(env: Env, account: Address, role: Symbol);

    fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>);

    /// Owner-only, non-vetoable council reset at Recovery delay.
    /// Public and slow — not a quiet theft path; recovers compromised canceller majority.
    fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a matured council reset. `executor=None` leaves execution open.
    fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    );

    /// Accepts pending ownership transfer of the governance contract.
    fn accept_ownership(env: Env);

    fn has_role(env: Env, account: Address, role: Symbol) -> bool;
}
