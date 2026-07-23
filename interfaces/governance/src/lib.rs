#![no_std]
#![allow(clippy::too_many_arguments)]

//! Client-only ABI mirror of the governance contract (production surface).
//!
//! `#[contractclient]` generates `GovernanceClient`. Matches deployed
//! entrypoints by ABI name. Test-only helpers (`execute_immediate`,
//! `set_controller`, `set_price_aggregator`) are excluded.

use common::types::{
    AssetOracleConfig, AssetOracleConfigInput, HubAssetKey, OracleTolerance, PositionLimits,
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
    pub cfg: AssetOracleConfigInput,
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
// `AssetOracleConfigOption`.
#[allow(clippy::large_enum_variant)]
pub enum AdminOperation {
    // Controller target
    SetSwapAggregator(Address),
    SetPriceAggregator(Address),
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
    /// Deploys the lending controller once and records its address. Owner only.
    /// Governance is the controller constructor admin.
    ///
    /// # Errors
    /// * `InvalidPoolTemplate` — `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` — controller address already stored.
    ///
    /// # Events
    /// * `DeployControllerEvent` — address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance holds every controller admin power after deploy.
    fn deploy_controller(env: Env, wasm_hash: BytesN<32>) -> Address;

    /// Stored controller address.
    ///
    /// # Errors
    /// * `PoolNotInitialized` — controller not deployed.
    fn controller(env: Env) -> Address;

    /// Deploys the price-aggregator once and records its address. Owner only.
    /// Governance is the aggregator constructor owner; if a controller exists,
    /// wires it immediately (Sensitive re-point still uses `SetPriceAggregator`).
    ///
    /// # Errors
    /// * `InvalidPoolTemplate` — `wasm_hash` is all-zero.
    /// * `PoolAlreadyDeployed` — aggregator address already stored.
    ///
    /// # Events
    /// * `DeployPriceAggregatorEvent` — address and wasm hash.
    ///
    /// # Security Warning
    /// * Governance holds every oracle admin power after deploy.
    fn deploy_price_aggregator(env: Env, wasm_hash: BytesN<32>) -> Address;

    /// Stored price-aggregator address.
    ///
    /// # Errors
    /// * `AggregatorNotSet` — aggregator not deployed.
    fn price_aggregator(env: Env) -> Address;

    /// Executes a ready non-self timelock operation and returns its result.
    /// `Some(executor)` requires `EXECUTOR` auth; `None` leaves execution open.
    /// Self-ops must use `execute_self`.
    ///
    /// # Errors
    /// * `InternalError` — `target` is this governance contract.
    /// * `TimelockOperationExpired` — past grace window.
    /// * OZ timelock rejects not-scheduled / not-ready; `EXECUTOR` gate when set.
    ///
    /// # Events
    /// * OZ timelock execute event; target emits its own.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready operation.
    fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val;

    /// Cancels a pending timelock operation. `CANCELLER`-gated.
    /// Recovery-tier ops and self-targeted role revocations are not cancellable.
    ///
    /// # Errors
    /// * `OperationNotCancellable` — Recovery op, or revoke of `canceller`'s own role.
    /// * Access-control / OZ timelock reject unknown canceller or not-pending.
    ///
    /// # Events
    /// * OZ timelock cancel event.
    fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>);

    /// Minimum timelock delay in ledgers.
    fn get_min_delay(env: Env) -> u32;

    /// Lifecycle state of a scheduled operation.
    fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState;

    /// Ledger when an operation becomes ready (`0` unset, `1` done).
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

    /// Resolves market oracle input to the `AssetOracleConfig` `propose` would
    /// schedule, including live probes. Read-only.
    ///
    /// # Errors
    /// * `BadLastTolerance`, `MathOverflow`.
    /// * `InvalidExchangeSrc`, `SpotOnlyNotProductionSafe`, `InvalidStalenessConfig`,
    ///   `InvalidSanityBounds`, `SanityBandTooWideForSingleSource`, `InvalidOracleDecimals`.
    /// * Live probe: `InvalidAsset`, `InvalidTicker`, `InvalidOracleBase`,
    ///   `InvalidOracleResolution`, `ReflectorHistoryEmpty`,
    ///   `TwapInsufficientObservations`, `PriceFeedStale`.
    fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: AssetOracleConfigInput,
    ) -> AssetOracleConfig;

    /// Resolves tolerance BPS to the `OracleTolerance` band `propose` would
    /// schedule. Read-only.
    ///
    /// # Errors
    /// * `BadLastTolerance` — outside allowed BPS range.
    /// * `MathOverflow` — band computation overflows.
    fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance;

    /// Schedules an `AdminOperation` and returns its operation id. `PROPOSER`-gated.
    /// Sensitive floor: upgrades, ownership transfers, `SetPriceAggregator`. Other
    /// ops use min delay. `TransferGovOwnership` requires the owner as proposer;
    /// `RevokeGovRole` may not target the proposer or the owner.
    ///
    /// # Errors
    /// * `NotAuthorized` — revoke self/owner, or non-owner proposes ownership transfer.
    /// * `PoolNotInitialized` / `AggregatorNotSet` — target not wired yet.
    /// * Via `resolve_op`: `InvalidPoolTemplate`, `InvalidTimelockDelay`, `InvalidRole`,
    ///   `InvalidAggregator`, `NotSmartContract`, `InvalidPositionLimits`,
    ///   `InvalidBorrowParams`, `WrongToken`, `InvalidAsset`, `BadLastTolerance`,
    ///   `InvalidExchangeSrc`, and live oracle-probe reverts.
    /// * Access-control / OZ timelock reject unknown proposer or duplicate schedule.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    fn propose(env: Env, proposer: Address, op: AdminOperation, salt: BytesN<32>) -> BytesN<32>;

    /// Halts the controller immediately. `GUARDIAN`-gated. Resume is timelocked
    /// `AdminOperation::Unpause` only.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert on pause.
    ///
    /// # Events
    /// * Controller pause event.
    fn pause(env: Env, caller: Address);

    /// Sets spoke listing `paused`/`frozen` immediately. `GUARDIAN`-gated.
    /// Tighten-only (`false → true` or stay); clearing rides timelocked
    /// `EditAssetInSpoke`.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`.
    /// * `AssetNotInSpoke`, `SpokeAssetFlagRelaxation` from the controller.
    ///
    /// # Events
    /// * Controller spoke-asset update event.
    fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    );

    /// Moves an asset oracle sanity band immediately. `ORACLE`-gated. Aggregator
    /// requires the new band to contain the live price.
    ///
    /// # Errors
    /// * Access-control rejects non-`ORACLE`.
    /// * `PairNotActive`, `InvalidSanityBounds`, `SanityBandTooWideForSingleSource`,
    ///   `SanityBoundViolated`, and feed-resolution errors from the aggregator.
    ///
    /// # Events
    /// * Aggregator `UpdateAssetOracleEvent`.
    fn set_sanity_band(env: Env, caller: Address, asset: Address, min_wad: i128, max_wad: i128);

    /// Creates a hub and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    fn create_hub(env: Env, caller: Address) -> u32;

    /// Creates a spoke and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    fn add_spoke(env: Env, caller: Address) -> u32;

    /// Revokes `GUARDIAN` or `ORACLE` immediately. Owner only. Other role
    /// revokes and all grants stay timelocked; canceller deadlock uses
    /// `propose_canceller_reset`.
    ///
    /// # Errors
    /// * `InvalidRole` — not `GUARDIAN`/`ORACLE`, or `account` does not hold it.
    /// * `NotAuthorized` — `account` is the owner (roles never revocable).
    ///
    /// # Events
    /// * Access-control role-revoke event.
    fn revoke_role_immediate(env: Env, account: Address, role: Symbol);

    /// Applies a ready governance-self op inline (upgrade, delay, roles,
    /// ownership, `SetPriceAggregator`). `Some(executor)` requires `EXECUTOR`;
    /// `None` leaves execution open.
    ///
    /// # Errors
    /// * `InternalError` — `op` does not target this contract.
    /// * `TimelockOperationExpired` — past grace window.
    /// * `InvalidTimelockDelay`, `InvalidRole`, `OwnerNotSet`, `InvalidAggregator`
    ///   on self-apply; OZ not-scheduled / not-ready.
    ///
    /// # Events
    /// * OZ timelock execute event plus role / ownership / upgrade events.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready self-operation.
    fn execute_self(env: Env, executor: Option<Address>, op: AdminOperation, salt: BytesN<32>);

    /// Schedules a non-vetoable canceller-council reset at Recovery delay. Owner only.
    ///
    /// # Errors
    /// * Owner gate via `#[only_owner]`.
    /// * OZ timelock rejects duplicate schedule.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32>;

    /// Executes a matured canceller-council reset. `Some(executor)` requires
    /// `EXECUTOR`; `None` leaves execution open.
    ///
    /// # Errors
    /// * `TimelockOperationExpired` — past grace window.
    /// * `InvalidRole` — EXECUTOR/CANCELLER overlap on a non-owner grant.
    /// * OZ not-scheduled / not-ready; `EXECUTOR` gate when set.
    ///
    /// # Events
    /// * OZ timelock execute event; access-control role grant/revoke events.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready reset.
    fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    );

    /// Accepts pending ownership transfer; pending owner must authorize.
    /// Migrates access-control admin and the five operational roles.
    ///
    /// # Errors
    /// * `OwnerNotSet` — no current owner.
    /// * OZ ownable rejects unauthorized or missing pending transfer.
    ///
    /// # Events
    /// * Ownership- and admin-transfer-completed; role grant/revoke events.
    fn accept_ownership(env: Env);

    /// Whether `account` holds `role`.
    fn has_role(env: Env, account: Address, role: Symbol) -> bool;
}
