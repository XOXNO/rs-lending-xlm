//! Timelock execution and query surface backed by `stellar-governance` storage.
//!
//! Operations are queued through typed `propose_*` entrypoints in `forward.rs`;
//! generic `Timelock::schedule` is not exposed. This module provides `execute`,
//! `cancel`, and query views over the OZ operation state machine.
//!
//! `execute` requires EXECUTOR auth when `executor` is `Some`; `None` keeps
//! execution open. `cancel` requires CANCELLER. Controller-targeted operations
//! call `execute_operation`; governance-self operations use typed inline
//! dispatch in `self_timelock.rs` because Soroban prohibits self-reentry.

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE};
use crate::storage::renew_governance_instance;
use crate::{constants, forward, validate, Governance, GovernanceArgs, GovernanceClient};
use common::errors::GenericError;
use controller_interface::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
};
use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env, Symbol, Val, Vec};
use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, get_min_delay, get_operation_ledger, get_operation_state,
    hash_operation, Operation, OperationState,
};

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

/// Rejects a zero minimum timelock delay, which would nullify the timelock.
pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay >= 1, GenericError::InvalidTimelockDelay);
}

/// Delay updates must not shorten the timelock window and cannot exceed 14 days.
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

#[contractimpl]
impl Governance {
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

    /// Cancels a pending operation. The caller must hold CANCELLER.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        cancel_operation(&env, &operation_id);
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

    /// Resolves a market oracle input to the `MarketOracleConfig` scheduled by
    /// `propose_configure_market_oracle`. Uses the proposer's validation and
    /// live oracle probes so simulation can replay the returned args at execute.
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig {
        forward::resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves tolerance BPS inputs to the `OraclePriceFluctuation` scheduled
    /// by `propose_edit_oracle_tolerance`. Uses the proposer's computation.
    pub fn resolve_oracle_tolerance(
        env: Env,
        first_tolerance: u32,
        last_tolerance: u32,
    ) -> OraclePriceFluctuation {
        validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        )
    }
}

#[cfg(test)]
#[path = "../tests/timelock.rs"]
mod tests;
