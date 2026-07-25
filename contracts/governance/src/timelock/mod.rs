//! Timelock lifecycle (`propose` / `execute` / `cancel`), immediate incident
//! brakes, and Recovery-tier canceller reset. Role gates and delay tiers match
//! ADR 0010. Executed and cancelled ops free their `OperationLedger` entry;
//! only pending ops occupy that storage.

mod immediate;
mod lifecycle;
mod recovery;
#[cfg(any(test, feature = "testing"))]
mod testing;
mod views;

use common::errors::GenericError;

use controller_interface::ControllerAdminClient;
use price_aggregator_interface::PriceAggregatorClient;

use soroban_sdk::{assert_with_error, vec, Address, BytesN, Env, IntoVal, Symbol, Vec};

use stellar_access::access_control;
use stellar_governance::timelock::{
    get_min_delay, get_operation_ledger, hash_operation, Operation, TimelockStorageKey,
};

use crate::access::EXECUTOR_ROLE;
use crate::op::resolve_op;
use crate::{constants, storage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelayTier {
    Standard,
    /// Upgrades, ownership transfers, and price-aggregator re-point.
    Sensitive,
    /// Non-vetoable council reset; longest delay.
    Recovery,
}

/// Delay for a tier: the configured minimum, raised to the tier's own floor.
pub(crate) fn operation_delay(env: &Env, tier: DelayTier) -> u32 {
    let min = get_min_delay(env);
    match tier {
        DelayTier::Standard => min,
        DelayTier::Sensitive => min.max(constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS),
        DelayTier::Recovery => min.max(constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS),
    }
}

/// # Errors
/// * `InvalidTimelockDelay` — a zero delay would make the timelock a no-op.
pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay != 0, GenericError::InvalidTimelockDelay);
}

// Non-decreasing, capped at 14 days.
/// Accepts a delay change only if it is non-decreasing and within the cap, so
/// governance can lengthen its own delay but never shorten it.
///
/// # Errors
/// * `InvalidTimelockDelay` — zero, below the current delay, or above the cap.
pub(crate) fn validate_delay_update(env: &Env, new_delay: u32) {
    require_nonzero_delay(env, new_delay);
    let current = get_min_delay(env);
    assert_with_error!(
        env,
        new_delay >= current && new_delay <= constants::TIMELOCK_MAX_DELAY_LEDGERS,
        GenericError::InvalidTimelockDelay
    );
}

/// Validates and writes the new minimum delay.
pub(crate) fn apply_update_delay(env: &Env, new_delay: u32) {
    validate_delay_update(env, new_delay);
    stellar_governance::timelock::set_min_delay(env, new_delay);
}

/// Requires auth and the `EXECUTOR` role when an executor is named. `None`
/// means the operation is open to any caller once matured.
pub(crate) fn authorize_executor(env: &Env, executor: Option<&Address>) {
    if let Some(exec) = executor {
        exec.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);
    }
}

/// Rejects an operation past its grace window, so a long-forgotten proposal
/// cannot be executed against a chain that has moved on.
///
/// # Errors
/// * `TimelockOperationExpired` — past `ready_ledger + grace`.
pub(crate) fn require_operation_not_expired(env: &Env, operation_id: &BytesN<32>) {
    let ready_ledger = get_operation_ledger(env, operation_id);
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

/// Builds the timelock `Operation` for an `AdminOperation` with predecessor `0`.
fn operation_for_admin_op(
    env: &Env,
    op: &crate::op::AdminOperation,
    salt: BytesN<32>,
) -> (Operation, DelayTier) {
    let resolved = resolve_op(env, op);
    (
        Operation {
            target: resolved.target,
            function: resolved.function,
            args: resolved.args,
            predecessor: BytesN::from_array(env, &[0u8; 32]),
            salt,
        },
        resolved.delay_tier,
    )
}

fn canceller_reset_operation(
    env: &Env,
    new_cancellers: &Vec<Address>,
    salt: BytesN<32>,
) -> Operation {
    Operation {
        target: env.current_contract_address(),
        function: Symbol::new(env, "reset_cancellers"),
        args: vec![env, new_cancellers.clone().into_val(env)],
        predecessor: BytesN::from_array(env, &[0u8; 32]),
        salt,
    }
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

/// Shared execute prep: renew → executor auth → expiry. Returns the operation id
/// so callers reuse it through execute / finish without re-hashing.
fn prepare_execute(env: &Env, executor: Option<&Address>, operation: &Operation) -> BytesN<32> {
    storage::renew_governance_instance(env);
    authorize_executor(env, executor);
    let operation_id = hash_operation(env, operation);
    require_operation_not_expired(env, &operation_id);
    operation_id
}

/// Removes the OZ `OperationLedger` entry and local sidecars after a successful
/// execute or cancel. Pending ops only occupy storage; `salt` uniquifies re-proposes.
/// Predecessor chaining is unsupported (`propose` always uses predecessor `0`).
fn finish_execute(env: &Env, operation_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&TimelockStorageKey::OperationLedger(operation_id.clone()));
    storage::clear_operation_sidecars(env, operation_id);
}

#[cfg(test)]
#[path = "../../tests/timelock.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/self_timelock.rs"]
mod self_timelock_tests;
