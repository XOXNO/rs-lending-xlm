//! Execution guards: governance state an operation is bound to when it is
//! proposed, so an emergency action taken in between voids the operation
//! instead of being undone by it; and the exclusion that keeps an `Unpause`
//! and an `UpgradeController` from being pending together.

use common::errors::GenericError;
use controller_interface::ControllerClient;

use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env, Vec};

use crate::op::AdminOperation;
use crate::storage::{self, ExclusiveKind, ExecutionGuard};

use super::operation_live;

/// Returns `op`'s exclusive kind and the kind it excludes, if any.
fn exclusion(op: &AdminOperation) -> Option<(ExclusiveKind, ExclusiveKind)> {
    match op {
        AdminOperation::Unpause => Some((ExclusiveKind::Unpause, ExclusiveKind::ControllerUpgrade)),
        AdminOperation::UpgradeController(_) => {
            Some((ExclusiveKind::ControllerUpgrade, ExclusiveKind::Unpause))
        }
        _ => None,
    }
}

/// Returns the recorded pending ids of `kind` that are still waiting or
/// ready and not expired.
fn live_pending(env: &Env, kind: ExclusiveKind) -> Vec<BytesN<32>> {
    let mut live = Vec::new(env);
    for id in storage::pending_exclusive(env, kind).iter() {
        if operation_live(env, &id) {
            live.push_back(id);
        }
    }
    live
}

/// Panics with `GenericError::ConflictingOperationPending` if `op` is an
/// `Unpause` while an `UpgradeController` is pending, or the reverse.
pub(crate) fn require_no_conflict(env: &Env, op: &AdminOperation) {
    if let Some((_, excluded)) = exclusion(op) {
        assert_with_error!(
            env,
            live_pending(env, excluded).is_empty(),
            GenericError::ConflictingOperationPending
        );
    }
}

/// Records the execution guard `op` needs under `operation_id`, if any, and
/// tracks `operation_id` as pending when `op` has an exclusive kind.
///
/// `Unpause` binds to the controller's current pause epoch and panics with
/// `GenericError::PauseEpochMismatch` while the controller is open.
pub(crate) fn record(env: &Env, operation_id: &BytesN<32>, op: &AdminOperation) {
    if let Some((kind, _)) = exclusion(op) {
        let mut pending = live_pending(env, kind);
        pending.push_back(operation_id.clone());
        storage::set_pending_exclusive(env, kind, &pending);
    }
    let guard = match op {
        AdminOperation::Unpause => ExecutionGuard::PauseEpoch(
            ControllerClient::new(env, &storage::get_controller(env))
                .get_pause_epoch()
                .unwrap_or_else(|| panic_with_error!(env, GenericError::PauseEpochMismatch)),
        ),
        _ => return,
    };
    storage::set_execution_guard(env, operation_id, &guard);
}

/// Panics if the state `operation_id` was bound to at proposal has moved.
/// `target` is the contract the operation invokes. An operation without a
/// recorded guard passes.
pub(crate) fn require_holds(env: &Env, operation_id: &BytesN<32>, target: &Address) {
    match storage::execution_guard(env, operation_id) {
        Some(ExecutionGuard::PauseEpoch(epoch)) => assert_with_error!(
            env,
            ControllerClient::new(env, target).get_pause_epoch() == Some(epoch),
            GenericError::PauseEpochMismatch
        ),
        None => {}
    }
}
