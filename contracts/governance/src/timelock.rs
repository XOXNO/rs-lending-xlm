//! Timelock lifecycle (`propose` / `execute` / `cancel`), immediate incident
//! brakes, and Recovery-tier canceller reset. Role gates and delay tiers match
//! ADR 0010.

use common::errors::GenericError;
use common::types::{AssetOracleConfig, AssetOracleConfigInput, HubAssetKey, OracleTolerance};

use controller_interface::ControllerAdminClient;
use price_aggregator_interface::PriceAggregatorClient;

use soroban_sdk::{
    assert_with_error, contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val, Vec,
};

use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, get_min_delay, get_operation_ledger, get_operation_state,
    hash_operation, schedule_operation, set_execute_operation, Operation, OperationState,
};
use stellar_macros::only_owner;

use crate::access::{
    self, CANCELLER_ROLE, EXECUTOR_ROLE, GUARDIAN_ROLE, ORACLE_ROLE, PROPOSER_ROLE,
};
use crate::op::{apply_self_op, resolve_op};
use crate::storage::renew_governance_instance;
use crate::{constants, storage, validate, Governance, GovernanceArgs, GovernanceClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelayTier {
    Standard,
    /// Upgrades, ownership transfers, and price-aggregator re-point.
    Sensitive,
    /// Non-vetoable council reset; longest delay.
    Recovery,
}

pub(crate) fn operation_delay(env: &Env, tier: DelayTier) -> u32 {
    let min = get_min_delay(env);
    match tier {
        DelayTier::Standard => min,
        DelayTier::Sensitive => min.max(constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS),
        DelayTier::Recovery => min.max(constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS),
    }
}

pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay != 0, GenericError::InvalidTimelockDelay);
}

// Non-decreasing, capped at 14 days.
pub(crate) fn validate_delay_update(env: &Env, new_delay: u32) {
    require_nonzero_delay(env, new_delay);
    let current = get_min_delay(env);
    assert_with_error!(
        env,
        new_delay >= current && new_delay <= constants::TIMELOCK_MAX_DELAY_LEDGERS,
        GenericError::InvalidTimelockDelay
    );
}

pub(crate) fn apply_update_delay(env: &Env, new_delay: u32) {
    validate_delay_update(env, new_delay);
    stellar_governance::timelock::set_min_delay(env, new_delay);
}

pub(crate) fn authorize_executor(env: &Env, executor: Option<&Address>) {
    if let Some(exec) = executor {
        exec.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);
    }
}

pub(crate) fn require_operation_not_expired(env: &Env, operation: &Operation) {
    let operation_id = hash_operation(env, operation);
    let ready_ledger = get_operation_ledger(env, &operation_id);
    if ready_ledger <= 1 {
        return;
    }

    let expires_at = ready_ledger.saturating_add(constants::TIMELOCK_OPERATION_GRACE_LEDGERS);
    assert_with_error!(
        env,
        env.ledger().sequence() <= expires_at,
        GenericError::TimelockOperationExpired
    );
}

fn controller_client(env: &Env) -> ControllerAdminClient<'_> {
    ControllerAdminClient::new(env, &storage::get_controller(env))
}

fn price_aggregator_client(env: &Env) -> PriceAggregatorClient<'_> {
    PriceAggregatorClient::new(env, &storage::get_price_aggregator(env))
}

fn begin_immediate(env: &Env, caller: &Address, role: &str) {
    storage::renew_governance_instance(env);
    caller.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, role), caller);
}

// Self-target execute is inline; Soroban blocks self-reentry.
fn begin_self_execute(env: &Env, executor: Option<&Address>, operation: &Operation) {
    renew_governance_instance(env);
    authorize_executor(env, executor);
    require_operation_not_expired(env, operation);
    set_execute_operation(env, operation);
}

fn resolve_market_oracle(
    env: &Env,
    asset: &Address,
    cfg: &AssetOracleConfigInput,
) -> AssetOracleConfig {
    let tolerance = validate::tolerance::validate_and_calculate_tolerances(env, cfg.tolerance_bps);
    validate::oracle_probe::validate_market_oracle_sources(env, asset, cfg, tolerance)
}

#[contractimpl]
impl Governance {
    /// Schedules an `AdminOperation` and returns its operation id. `PROPOSER`-gated.
    /// Sensitive floor: upgrades, ownership transfers, `SetPriceAggregator`. Other
    /// ops use min delay. `TransferGovOwnership` requires the owner as proposer;
    /// `RevokeGovRole` may not target the proposer or the owner.
    ///
    /// # Errors
    /// * `NotAuthorized` — revoke self/owner, or non-owner proposes ownership transfer.
    /// * `PoolNotInitialized` / `AggregatorNotSet` — target not wired yet.
    /// * Via `resolve_op`: `InvalidWasmHash`, `InvalidTimelockDelay`, `InvalidRole`,
    ///   `InvalidAggregator`, `NotSmartContract`, `InvalidPositionLimits`,
    ///   `InvalidBorrowParams`, `WrongToken`, `InvalidAsset`, `BadLastTolerance`,
    ///   `InvalidExchangeSrc`, and live oracle-probe reverts.
    /// * Access-control / OZ timelock reject unknown proposer or duplicate schedule.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    pub fn propose(
        env: Env,
        proposer: Address,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_immediate(&env, &proposer, PROPOSER_ROLE);
        match &op {
            // A proposer may revoke anyone's role except its own; the owner's
            // roles are never revocable. The owner check is re-enforced at apply.
            crate::op::AdminOperation::RevokeGovRole(args) => {
                assert_with_error!(&env, args.account != proposer, GenericError::NotAuthorized);
                assert_with_error!(
                    &env,
                    args.account != access::owner_or_panic(&env),
                    GenericError::NotAuthorized
                );
            }
            // Only the owner may initiate an ownership transfer; any canceller
            // can still veto it during the timelock.
            crate::op::AdminOperation::TransferGovOwnership(_) => {
                assert_with_error!(
                    &env,
                    proposer == access::owner_or_panic(&env),
                    GenericError::NotAuthorized
                );
            }
            _ => {}
        }
        let (target, function, args, delay_tier) = resolve_op(&env, &op);
        let delay = operation_delay(&env, delay_tier);
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        let operation_id = schedule_operation(&env, &operation, delay);
        // Record the target and role so `cancel` can enforce the self-veto guard.
        if let crate::op::AdminOperation::RevokeGovRole(args) = &op {
            storage::mark_role_revocation_target(&env, &operation_id, &args.account, &args.role);
        }
        operation_id
    }

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
    pub fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val {
        renew_governance_instance(&env);
        authorize_executor(&env, executor.as_ref());
        assert_with_error!(
            &env,
            target != env.current_contract_address(),
            GenericError::InternalError
        );
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        require_operation_not_expired(&env, &operation);
        execute_operation(&env, &operation)
    }

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
    pub fn execute_self(
        env: Env,
        executor: Option<Address>,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) {
        let (target, function, args, _) = resolve_op(&env, &op);
        assert_with_error!(
            env,
            target == env.current_contract_address(),
            GenericError::InternalError
        );
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        begin_self_execute(&env, executor.as_ref(), &operation);
        apply_self_op(&env, &op);
    }

    /// Cancels a pending timelock operation. `CANCELLER`-gated.
    /// Recovery-tier ops and self-targeted role revocations are not cancellable.
    ///
    /// # Errors
    /// * `OperationNotCancellable` — Recovery op, or revoke of `canceller`'s own role.
    /// * Access-control / OZ timelock reject unknown canceller or not-pending.
    ///
    /// # Events
    /// * OZ timelock cancel event.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        // Recovery-tier operations are non-vetoable — they exist precisely to
        // override a captured canceller council.
        assert_with_error!(
            &env,
            !storage::is_recovery_op(&env, &operation_id),
            GenericError::OperationNotCancellable
        );
        // A role revocation cannot be vetoed by its own target — no one blocks
        // their own removal. Every other pending operation, including a
        // revocation of another canceller, stays vetoable, so the independent
        // cancellers remain a real check on a rogue proposer (or owner). A
        // colluding-canceller deadlock is broken by the non-vetoable Recovery
        // tier (`propose_canceller_reset`), not by suspending the veto here.
        if let Some((target, _role)) = storage::role_revocation_target(&env, &operation_id) {
            assert_with_error!(
                &env,
                target != canceller,
                GenericError::OperationNotCancellable
            );
        }
        cancel_operation(&env, &operation_id);
    }

    /// Halts the controller immediately. `GUARDIAN`-gated. Resume is timelocked
    /// `AdminOperation::Unpause` only.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert on pause.
    ///
    /// # Events
    /// * Controller pause event.
    pub fn pause(env: Env, caller: Address) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).pause();
    }

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
    pub fn set_spoke_asset_flags(
        env: Env,
        caller: Address,
        spoke_id: u32,
        hub_asset: HubAssetKey,
        paused: bool,
        frozen: bool,
    ) {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).set_spoke_asset_flags(&spoke_id, &hub_asset, &paused, &frozen);
    }

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
    pub fn set_sanity_band(
        env: Env,
        caller: Address,
        asset: Address,
        min_wad: i128,
        max_wad: i128,
    ) {
        begin_immediate(&env, &caller, ORACLE_ROLE);
        price_aggregator_client(&env).set_sanity_band(&asset, &min_wad, &max_wad);
    }

    /// Creates a hub and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn create_hub(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).create_hub()
    }

    /// Creates a spoke and returns its id. `GUARDIAN`-gated. Listings still ride
    /// the timelock.
    ///
    /// # Errors
    /// * Access-control rejects non-`GUARDIAN`; controller may revert.
    pub fn add_spoke(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).add_spoke()
    }

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
    #[only_owner]
    pub fn revoke_role_immediate(env: Env, account: Address, role: Symbol) {
        assert_with_error!(
            &env,
            role == Symbol::new(&env, GUARDIAN_ROLE) || role == Symbol::new(&env, ORACLE_ROLE),
            GenericError::InvalidRole
        );
        access::apply_revoke_role(&env, &account, &role);
    }

    /// Minimum timelock delay in ledgers.
    pub fn get_min_delay(env: Env) -> u32 {
        get_min_delay(&env)
    }

    /// Lifecycle state of a scheduled operation.
    pub fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        get_operation_state(&env, &operation_id)
    }

    /// Ledger when an operation becomes ready (`0` unset, `1` done).
    pub fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32 {
        get_operation_ledger(&env, &operation_id)
    }

    /// Deterministic operation id for the given fields.
    pub fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        hash_operation(&env, &operation)
    }

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
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: AssetOracleConfigInput,
    ) -> AssetOracleConfig {
        resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves tolerance BPS to the `OracleTolerance` band `propose` would
    /// schedule. Read-only.
    ///
    /// # Errors
    /// * `BadLastTolerance` — outside allowed BPS range.
    /// * `MathOverflow` — band computation overflows.
    pub fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OracleTolerance {
        validate::tolerance::validate_and_calculate_tolerances(&env, tolerance)
    }

    /// Schedules a non-vetoable canceller-council reset at Recovery delay. Owner only.
    ///
    /// # Errors
    /// * Owner gate via `#[only_owner]`.
    /// * OZ timelock rejects duplicate schedule.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    #[only_owner]
    pub fn propose_canceller_reset(
        env: Env,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let gov = env.current_contract_address();
        let operation = Operation {
            target: gov,
            function: Symbol::new(&env, "reset_cancellers"),
            args: vec![&env, new_cancellers.into_val(&env)],
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        let delay = operation_delay(&env, DelayTier::Recovery);
        let id = schedule_operation(&env, &operation, delay);
        storage::mark_recovery_op(&env, &id);
        id
    }

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
    pub fn execute_canceller_reset(
        env: Env,
        executor: Option<Address>,
        new_cancellers: Vec<Address>,
        salt: BytesN<32>,
    ) {
        let gov = env.current_contract_address();
        let operation = Operation {
            target: gov,
            function: Symbol::new(&env, "reset_cancellers"),
            args: vec![&env, new_cancellers.clone().into_val(&env)],
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        begin_self_execute(&env, executor.as_ref(), &operation);
        access::apply_canceller_reset(&env, &new_cancellers);
        storage::clear_recovery_op(&env, &hash_operation(&env, &operation));
    }
}

/// Test-only immediate executor; excluded from production WASM.
#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    pub fn execute_immediate(env: Env, caller: Address, op: crate::op::AdminOperation) -> Val {
        storage::renew_governance_instance(&env);
        caller.require_auth();
        match &op {
            crate::op::AdminOperation::ConfigureMarketOracle(_)
            | crate::op::AdminOperation::EditOracleTolerance(_) => {
                stellar_access::access_control::ensure_role(
                    &env,
                    &Symbol::new(&env, crate::access::ORACLE_ROLE),
                    &caller,
                );
            }
            _ => {
                let owner = stellar_access::ownable::get_owner(&env)
                    .unwrap_or_else(|| panic!("Owner not set"));
                assert_eq!(caller, owner, "not owner");
            }
        }
        let (target, function, args, _) = resolve_op(&env, &op);
        if target == env.current_contract_address() {
            apply_self_op(&env, &op);
            ().into_val(&env)
        } else {
            env.invoke_contract(&target, &function, args)
        }
    }
}

#[cfg(test)]
#[path = "../tests/timelock.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/self_timelock.rs"]
mod self_timelock_tests;
