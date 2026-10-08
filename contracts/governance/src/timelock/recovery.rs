//! Scheduling and execution of the canceller-set recovery operation, a
//! self-targeting reset of the timelock's canceller list that uses the
//! `Recovery` delay tier and is marked so it cannot be cancelled through the
//! normal cancellation path.

use soroban_sdk::{Address, BytesN, Env, Vec};

use stellar_governance::timelock::{schedule_operation, set_execute_operation};

use crate::access::{self};
use crate::storage;
use crate::timelock::*;

/// Schedules a canceller-set reset to `new_cancellers` using the `Recovery` delay
/// tier, marks the resulting operation as a recovery operation bound to the
/// current owner epoch, and returns its id. Rejects a list that, with the owner's seat, exceeds `MAX_CANCELLERS`.
/// An expired reset with the same id is cleared first.
pub(crate) fn propose_canceller_reset(
    env: &Env,
    new_cancellers: &Vec<Address>,
    salt: BytesN<32>,
) -> BytesN<32> {
    access::require_canceller_count_within_cap(env, new_cancellers.len().saturating_add(1));
    let operation = canceller_reset_operation(env, new_cancellers, salt);
    let delay = operation_delay(env, DelayTier::Recovery);
    clear_expired_operation(env, &operation);
    let id = schedule_operation(env, &operation, delay);
    storage::mark_recovery_op(env, &id);
    storage::mark_proposal_owner_epoch(env, &id);
    id
}

/// Executes a previously scheduled canceller-set reset to `new_cancellers` once
/// its delay has elapsed and it has not expired, replacing the canceller set and
/// clearing the operation's scheduled state. A reset proposed before a later
/// ownership handover reverts `NotAuthorized`.
pub(crate) fn execute_canceller_reset(
    env: &Env,
    executor: Option<Address>,
    new_cancellers: &Vec<Address>,
    salt: BytesN<32>,
) {
    let operation = canceller_reset_operation(env, new_cancellers, salt);
    let operation_id = prepare_execute(env, executor.as_ref(), &operation);
    set_execute_operation(env, &operation);
    access::apply_canceller_reset(env, new_cancellers);
    finish_execute(env, &operation_id);
}
