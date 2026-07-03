//! Timelocked governance operations and immediate pause controls.

use common::errors::GenericError;
use common::types::{MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation};
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

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
use crate::storage::renew_governance_instance;
use crate::{constants, storage, validate, Governance, GovernanceArgs, GovernanceClient};

/// Standard vs elevated schedule delays for governance operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelayTier {
    Standard,
    /// Governance/controller upgrade and ownership transfer proposals.
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
    assert_with_error!(env, delay >= 1, GenericError::InvalidTimelockDelay);
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

/// Shared proposal auth and TTL renewal.
fn begin_proposal(env: &Env, proposer: &Address) {
    storage::renew_governance_instance(env);
    proposer.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, PROPOSER_ROLE), proposer);
}

/// Advances self-targeted timelock operation inline; Soroban blocks self-reentry.
fn begin_self_execute(env: &Env, executor: Option<Address>, operation: &Operation) {
    renew_governance_instance(env);
    authorize_executor(env, executor.as_ref());
    require_operation_not_expired(env, operation);
    set_execute_operation(env, operation);
}

/// Builds controller oracle config from proposed input.
fn resolve_market_oracle(
    env: &Env,
    asset: &Address,
    cfg: &MarketOracleConfigInput,
) -> common::types::MarketOracleConfig {
    let tolerance = validate::tolerance::validate_and_calculate_tolerances(env, cfg.tolerance_bps);
    validate::oracle_probe::validate_market_oracle_sources(env, asset, cfg, tolerance)
}

#[contractimpl]
impl Governance {
    pub fn propose(
        env: Env,
        proposer: Address,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        let (target, function, args, delay_tier) = crate::op::resolve_op(&env, &op);
        let delay = operation_delay(&env, delay_tier);
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        let operation_id = schedule_operation(&env, &operation, delay);
        // Role revocations must not be cancellable, otherwise a rogue role holder
        // could veto their own removal and permanently freeze governance.
        if matches!(op, crate::op::AdminOperation::RevokeGovRole(_)) {
            storage::mark_uncancellable(&env, &operation_id);
        }
        operation_id
    }

    /// Executes a ready controller operation. When `executor` is `Some`, that
    /// address must hold EXECUTOR and authorize; `None` allows open execution.
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

    /// Executes a ready governance self-call inline.
    pub fn execute_self(
        env: Env,
        executor: Option<Address>,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) {
        let (target, function, args, _) = crate::op::resolve_op(&env, &op);
        assert!(target == env.current_contract_address());
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        begin_self_execute(&env, executor, &operation);
        crate::op::apply_self_op(&env, &op);
    }

    /// Cancels a pending operation. The caller must hold CANCELLER.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        assert_with_error!(
            &env,
            !storage::is_uncancellable(&env, &operation_id),
            GenericError::OperationNotCancellable
        );
        cancel_operation(&env, &operation_id);
    }

    /// Halt the controller immediately; owner-gated and not timelocked.
    #[only_owner]
    pub fn pause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).pause();
    }

    /// Resume the controller immediately; owner-gated.
    #[only_owner]
    pub fn unpause(env: Env) {
        storage::renew_governance_instance(&env);
        controller_client(&env).unpause();
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

    /// Resolves scheduled market oracle args using live probes.
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig {
        resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves scheduled tolerance bands.
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
        match &op {
            crate::op::AdminOperation::ConfigureMarketOracle(_)
            | crate::op::AdminOperation::EditOracleTolerance(_) => {
                caller.require_auth();
                stellar_access::access_control::ensure_role(
                    &env,
                    &Symbol::new(&env, crate::access::ORACLE_ROLE),
                    &caller,
                );
            }
            _ => {
                caller.require_auth();
                let owner = stellar_access::ownable::get_owner(&env)
                    .unwrap_or_else(|| panic!("Owner not set"));
                assert_eq!(caller, owner, "not owner");
            }
        }
        let (target, function, args, _) = crate::op::resolve_op(&env, &op);
        if target == env.current_contract_address() {
            crate::op::apply_self_op(&env, &op);
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
