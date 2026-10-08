//! Execution guards: governance state an operation is bound to when it is
//! proposed, so an emergency action taken in between voids the operation
//! instead of being undone by it.

use common::errors::GenericError;
use controller_interface::ControllerClient;

use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env};

use crate::op::AdminOperation;
use crate::storage::{self, ExecutionGuard};

/// Records the execution guard `op` needs under `operation_id`, if any.
///
/// `Unpause` binds to the controller's current pause epoch and panics with
/// `GenericError::PauseEpochMismatch` while the controller is open.
pub(crate) fn record(env: &Env, operation_id: &BytesN<32>, op: &AdminOperation) {
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
