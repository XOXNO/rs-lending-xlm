//! Timelocked governance operations and immediate pause controls.

use common::errors::GenericError;
use common::types::{
    HubAssetKey, MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
};

use controller_interface::ControllerAdminClient;

#[cfg(any(test, feature = "testing"))]
use soroban_sdk::IntoVal;
use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env, Symbol, Val, Vec};

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

// ################## TYPES ##################

/// Standard vs elevated schedule delays for governance operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelayTier {
    Standard,
    /// Contract upgrade (governance, controller, or pool) and ownership
    /// transfer proposals.
    Sensitive,
}

/// Ledger delay used when scheduling an operation at the given tier.
pub(crate) fn operation_delay(env: &Env, tier: DelayTier) -> u32 {
    let min = get_min_delay(env);
    match tier {
        DelayTier::Standard => min,
        DelayTier::Sensitive => min.max(constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS),
    }
}

/// Rejects zero timelock delay.
pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay != 0, GenericError::InvalidTimelockDelay);
}

/// Requires non-decreasing delay, capped at 14 days.
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

/// Shared auth for role-gated operations: TTL renewal, caller auth, role check.
fn begin_immediate(env: &Env, caller: &Address, role: &str) {
    storage::renew_governance_instance(env);
    caller.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, role), caller);
}

/// Advances self-targeted timelock operation inline; Soroban blocks self-reentry.
fn begin_self_execute(env: &Env, executor: Option<&Address>, operation: &Operation) {
    renew_governance_instance(env);
    authorize_executor(env, executor);
    require_operation_not_expired(env, operation);
    set_execute_operation(env, operation);
}

/// Builds controller oracle config from proposed input.
fn resolve_market_oracle(
    env: &Env,
    asset: &Address,
    cfg: &MarketOracleConfigInput,
) -> MarketOracleConfig {
    let tolerance = validate::tolerance::validate_and_calculate_tolerances(env, cfg.tolerance_bps);
    validate::oracle_probe::validate_market_oracle_sources(env, asset, cfg, tolerance)
}

#[contractimpl]
impl Governance {
    /// Validates and schedules an `AdminOperation` on the timelock, returning
    /// its operation id. Contract upgrades and ownership transfers schedule at
    /// the elevated Sensitive delay; all other operations use the min delay.
    ///
    /// # Arguments
    /// * `proposer` - must hold the `PROPOSER` role and authorize.
    /// * `op` - the operation; its inputs are validated per variant before
    ///   scheduling.
    /// * `salt` - disambiguates otherwise-identical operations.
    ///
    /// # Errors
    /// * `PoolNotInitialized` - a controller-targeted operation is proposed
    ///   before the controller is deployed.
    /// * Per-operation input validation (via `op::resolve_op`), e.g.
    ///   `InvalidPoolTemplate` (zero wasm hash), `InvalidTimelockDelay` (delay
    ///   update), `InvalidRole` (unknown governance role), `InvalidAggregator`
    ///   / `NotSmartContract` (address is not a deployed contract),
    ///   `InvalidPositionLimits`, `InvalidBorrowParams`, `WrongToken` /
    ///   `InvalidAsset` (market creation), `BadLastTolerance` /
    ///   `InvalidExchangeSrc` and live oracle-probe reverts (oracle config).
    /// * The `PROPOSER` role check and duplicate-schedule rejection are
    ///   enforced by the access-control and OZ timelock libraries.
    ///
    /// # Events
    /// * A timelock schedule event emitted by the OZ governance library.
    pub fn propose(
        env: Env,
        proposer: Address,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_immediate(&env, &proposer, PROPOSER_ROLE);
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
        // Record the target and role so `cancel` can enforce the self-veto and
        // CANCELLER-revocation veto-immunity guards.
        if let crate::op::AdminOperation::RevokeGovRole(args) = &op {
            storage::mark_role_revocation_target(&env, &operation_id, &args.account, &args.role);
        }
        operation_id
    }

    /// Executes a ready timelock operation against `target` (a non-self
    /// contract, typically the controller) and returns its result.
    ///
    /// # Arguments
    /// * `executor` - `Some(addr)` gates execution on the `EXECUTOR` role and
    ///   that address's authorization; `None` leaves execution open.
    ///
    /// # Errors
    /// * `InternalError` - `target` is the governance contract itself
    ///   (self-operations must go through `execute_self`).
    /// * `TimelockOperationExpired` - the operation is past its grace window.
    /// * The `EXECUTOR` role check and the not-scheduled / not-ready reverts are
    ///   enforced by the OZ timelock library.
    ///
    /// # Events
    /// * A timelock execute event (OZ governance library); the invoked target
    ///   entrypoint emits its own events.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready operation; the
    ///   timelock schedule and readiness gate are the operative control, not
    ///   the caller identity.
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

    /// Executes a ready governance-self operation (upgrade, delay update, role
    /// grant/revoke, or ownership transfer) inline once its timelock matures.
    ///
    /// # Arguments
    /// * `executor` - `Some(addr)` gates on the `EXECUTOR` role and that
    ///   address's authorization; `None` leaves execution open.
    /// * `op` - must be a governance-self variant.
    ///
    /// # Errors
    /// * Panics if `op` does not target the governance contract itself.
    /// * `TimelockOperationExpired` - the operation is past its grace window.
    /// * Self-application reverts: `InvalidTimelockDelay` (delay update),
    ///   `InvalidRole` (unknown/no-op role change or executor/canceller
    ///   overlap), or `OwnerNotSet` (ownership transfer without an owner).
    ///
    /// # Events
    /// * A timelock execute event plus the applied operation's own events
    ///   (role grant/revoke, ownership transfer, or upgrade).
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready self-operation;
    ///   the timelock schedule and readiness gate are the operative control.
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

    /// Cancels a pending timelock operation.
    ///
    /// # Arguments
    /// * `canceller` - must hold the `CANCELLER` role and authorize.
    ///
    /// # Errors
    /// * `OperationNotCancellable` - the operation revokes `canceller`'s own
    ///   role (a role holder cannot veto their own removal).
    /// * The `CANCELLER` role check and the not-pending reject are enforced by
    ///   the access-control and OZ timelock libraries.
    ///
    /// # Events
    /// * A timelock cancel event emitted by the OZ governance library.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        // A role revocation cannot be vetoed by its own target, and a CANCELLER
        // revocation cannot be vetoed by any canceller. The latter blocks
        // colluding cancellers from cross-vetoing each other's removal and
        // freezing governance; the owner ejects cancellers through the timelock.
        // Other role revocations keep only the self-veto block, so cancellers
        // can still veto a malicious revocation of another role.
        if let Some((target, role)) = storage::role_revocation_target(&env, &operation_id) {
            let revokes_canceller = role == Symbol::new(&env, CANCELLER_ROLE);
            assert_with_error!(
                &env,
                !revokes_canceller && target != canceller,
                GenericError::OperationNotCancellable
            );
        }
        cancel_operation(&env, &operation_id);
    }

    /// Emergency brake: halts the controller immediately, bypassing the
    /// timelock. Owner-gated.
    ///
    /// # Errors
    /// * Owner authorization is enforced by `#[only_owner]`; the controller's
    ///   `pause` may revert per its own rules.
    ///
    /// # Events
    /// * The controller emits its own pause event.
    #[only_owner]
    pub fn pause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).pause();
    }

    /// Resumes the controller immediately, bypassing the timelock. Owner-gated.
    ///
    /// # Errors
    /// * Owner authorization is enforced by `#[only_owner]`; the controller's
    ///   `unpause` may revert per its own rules.
    ///
    /// # Events
    /// * The controller emits its own unpause event.
    #[only_owner]
    pub fn unpause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).unpause();
    }

    /// Sets a spoke listing's `paused`/`frozen` flags immediately, bypassing
    /// the timelock. Guardian incident brake for one listing.
    ///
    /// # Arguments
    /// * `caller` - must hold the `GUARDIAN` role and authorize.
    ///
    /// # Errors
    /// * The `GUARDIAN` role check is enforced here; `AssetNotInSpoke`
    ///   propagates from the controller.
    ///
    /// # Events
    ///
    /// Refer to controller `update_spoke_asset` events.
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

    /// Moves an asset oracle's sanity band immediately, bypassing the
    /// timelock. Bot incident path for band exits; the controller proves the
    /// new band contains the current live price.
    ///
    /// # Arguments
    /// * `caller` - must hold the `ORACLE` role and authorize.
    ///
    /// # Errors
    /// * The `ORACLE` role check is enforced here; `PairNotActive`,
    ///   `InvalidSanityBounds`, `SanityBandTooWideForSingleSource`,
    ///   `SanityBoundViolated`, and feed-resolution errors propagate from the
    ///   controller.
    ///
    /// # Events
    /// * The controller emits `UpdateAssetOracleEvent`.
    pub fn set_oracle_sanity_bounds(
        env: Env,
        caller: Address,
        asset: Address,
        min_wad: i128,
        max_wad: i128,
    ) {
        begin_immediate(&env, &caller, ORACLE_ROLE);
        controller_client(&env).set_oracle_sanity_bounds(&asset, &min_wad, &max_wad);
    }

    /// Creates a hub immediately, bypassing the timelock, and returns its id.
    /// Safe instant: the new registry entry is inert until assets are listed
    /// through the timelocked path.
    ///
    /// # Arguments
    /// * `caller` - must hold the `GUARDIAN` role and authorize.
    pub fn create_hub(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).create_hub()
    }

    /// Creates a spoke immediately, bypassing the timelock, and returns its
    /// id. Safe instant: listings on it still ride the timelock.
    ///
    /// # Arguments
    /// * `caller` - must hold the `GUARDIAN` role and authorize.
    pub fn add_spoke(env: Env, caller: Address) -> u32 {
        begin_immediate(&env, &caller, GUARDIAN_ROLE);
        controller_client(&env).add_spoke()
    }

    /// Revokes `GUARDIAN` or `ORACLE` immediately, bypassing the timelock.
    /// Owner-gated emergency de-authorization: stripping a compromised
    /// immediate-role key must be at least as fast as the powers it holds.
    /// Restricted to the immediate incident roles — `PROPOSER`/`EXECUTOR`/
    /// `CANCELLER` revocations stay timelocked so a compromised owner key
    /// cannot instantly strip the independent cancellers and leave a
    /// malicious pending proposal without a veto. Grants stay timelocked.
    ///
    /// # Errors
    /// * `InvalidRole` - role is not `GUARDIAN`/`ORACLE`, or `account` does
    ///   not hold it.
    ///
    /// # Events
    /// * A role-revoke event from the access-control library.
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

    /// Current lifecycle state of an operation.
    pub fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        get_operation_state(&env, &operation_id)
    }

    /// Ledger at which an operation becomes ready (`0` unset, `1` done).
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

    /// Resolves a market oracle input to the `MarketOracleConfig` the matching
    /// proposer would schedule, running the same live oracle probes; read-only.
    ///
    /// # Errors
    /// * Tolerance validation: `BadLastTolerance` or `MathOverflow`.
    /// * Oracle shape/config: `InvalidExchangeSrc`, `SpotOnlyNotProductionSafe`,
    ///   `InvalidStalenessConfig`, `InvalidSanityBounds`,
    ///   `SanityBandTooWideForSingleSource`, or `InvalidOracleDecimals`.
    /// * Live probe: `InvalidAsset`, `InvalidTicker`, `InvalidOracleBase`,
    ///   `InvalidOracleResolution`, `ReflectorHistoryEmpty`,
    ///   `TwapInsufficientObservations`, or `PriceFeedStale`.
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig {
        resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves a tolerance-BPS input to the `OraclePriceFluctuation` band the
    /// matching proposer would schedule; read-only.
    ///
    /// # Errors
    /// * `BadLastTolerance` - `tolerance` is outside the allowed BPS range.
    /// * `MathOverflow` - band computation overflows.
    pub fn resolve_oracle_tolerance(env: Env, tolerance: u32) -> OraclePriceFluctuation {
        validate::tolerance::validate_and_calculate_tolerances(&env, tolerance)
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
