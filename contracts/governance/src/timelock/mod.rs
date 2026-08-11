//! Timelock support shared by the governance contract: delay-tier selection,
//! operation construction, expiry checks, and the clients and helpers used by the
//! `immediate`, `lifecycle`, `recovery`, and `views` submodules.

pub(crate) mod immediate;
pub(crate) mod lifecycle;
pub(crate) mod recovery;
#[cfg(any(test, feature = "testing"))]
mod testing;
pub(crate) mod views;

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

/// Classifies an operation by the minimum delay it must wait before execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DelayTier {
    Standard,

    Sensitive,

    Recovery,
}

/// Returns the delay, in ledgers, required for operations in `tier`. `Standard`
/// uses the configured minimum delay; `Sensitive` and `Recovery` use that minimum
/// raised to their respective floor constants.
pub(crate) fn operation_delay(env: &Env, tier: DelayTier) -> u32 {
    let min = get_min_delay(env);
    match tier {
        DelayTier::Standard => min,
        DelayTier::Sensitive => min.max(constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS),
        DelayTier::Recovery => min.max(constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS),
    }
}

/// Panics with `GenericError::InvalidTimelockDelay` if `delay` is zero.
pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay != 0, GenericError::InvalidTimelockDelay);
}

/// Validates that `new_delay` is nonzero, at least the current minimum delay, and
/// at most `constants::TIMELOCK_MAX_DELAY_LEDGERS`. Panics with
/// `GenericError::InvalidTimelockDelay` otherwise.
pub(crate) fn validate_delay_update(env: &Env, new_delay: u32) {
    require_nonzero_delay(env, new_delay);
    let current = get_min_delay(env);
    assert_with_error!(
        env,
        new_delay >= current && new_delay <= constants::TIMELOCK_MAX_DELAY_LEDGERS,
        GenericError::InvalidTimelockDelay
    );
}

/// Validates `new_delay` and, if valid, sets it as the timelock's minimum delay.
pub(crate) fn apply_update_delay(env: &Env, new_delay: u32) {
    validate_delay_update(env, new_delay);
    stellar_governance::timelock::set_min_delay(env, new_delay);
}

/// When `Some(exec)`, requires `exec` auth + `EXECUTOR_ROLE`. When `None`,
/// performs no auth or role check (anyone may drive execution of a ready op).
pub(crate) fn authorize_executor(env: &Env, executor: Option<&Address>) {
    if let Some(exec) = executor {
        exec.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);
    }
}

/// Panics with `GenericError::TimelockOperationExpired` if `operation_id` was
/// scheduled and its grace-period deadline has passed. An operation with a
/// ready-ledger of 0 or 1 is treated as not yet scheduled or not tracked, and
/// always passes.
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

/// Resolves `op` into a timelock `Operation` (target, function, args, zero
/// predecessor, and `salt`) together with its delay tier.
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

/// Builds the timelock `Operation` for resetting the canceller set to
/// `new_cancellers`, targeting this contract's `reset_cancellers` function.
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

/// Returns a client for the controller contract at the address stored in this
/// contract's storage.
fn controller_client(env: &Env) -> ControllerAdminClient<'_> {
    ControllerAdminClient::new(env, &storage::get_controller(env))
}

/// Returns a client for the price aggregator contract at the address stored in
/// this contract's storage.
fn price_aggregator_client(env: &Env) -> PriceAggregatorClient<'_> {
    PriceAggregatorClient::new(env, &storage::get_price_aggregator(env))
}

/// Renews the governance instance's storage TTL, requires `caller`'s
/// authorization, and requires `caller` to hold `role`.
fn begin_immediate(env: &Env, caller: &Address, role: &str) {
    storage::renew_governance_instance(env);
    caller.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, role), caller);
}

/// Renews the governance instance's storage TTL, authorizes `executor` if
/// present, computes `operation`'s id, and checks that the operation has not
/// expired. Returns the operation id.
fn prepare_execute(env: &Env, executor: Option<&Address>, operation: &Operation) -> BytesN<32> {
    storage::renew_governance_instance(env);
    authorize_executor(env, executor);
    let operation_id = hash_operation(env, operation);
    require_operation_not_expired(env, &operation_id);
    operation_id
}

/// Removes the operation's scheduled-ledger entry and clears any sidecar state
/// (such as recovery or role-revocation markers) associated with `operation_id`.
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
