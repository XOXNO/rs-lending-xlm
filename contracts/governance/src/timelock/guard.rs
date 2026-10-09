//! Execution guards: governance state an operation is bound to when it is
//! proposed, so an emergency action taken in between voids the operation
//! instead of being undone by it; and the single pending-controller-upgrade
//! slot that keeps an `Unpause` from executing ahead of an upgrade.

use common::errors::{GenericError, OracleError};
use controller_interface::ControllerClient;
use price_aggregator_interface::PriceAggregatorClient;

use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env};

use crate::op::AdminOperation;
use crate::storage::{self, ExecutionGuard};

use super::operation_live;

/// Returns whether a proposed `UpgradeController` is waiting or ready and
/// not expired.
fn controller_upgrade_pending(env: &Env) -> bool {
    storage::pending_controller_upgrade(env).is_some_and(|id| operation_live(env, &id))
}

/// Panics with `GenericError::ConflictingOperationPending` if `op` is an
/// `UpgradeController` while another one is pending; cancel it first.
pub(crate) fn require_no_pending_upgrade(env: &Env, op: &AdminOperation) {
    if let AdminOperation::UpgradeController(_) = op {
        assert_with_error!(
            env,
            !controller_upgrade_pending(env),
            GenericError::ConflictingOperationPending
        );
    }
}

/// Records the execution guard `op` needs under `operation_id`, if any, and
/// fills the pending-controller-upgrade slot for an `UpgradeController`.
///
/// `Unpause` binds to the controller's current pause epoch and panics with
/// `GenericError::PauseEpochMismatch` while the controller is open.
/// `ConfigureAssetOracle` binds to its key and the proposal ledger.
pub(crate) fn record(env: &Env, operation_id: &BytesN<32>, op: &AdminOperation) {
    if let AdminOperation::UpgradeController(_) = op {
        storage::set_pending_controller_upgrade(env, operation_id);
    }
    let guard = match op {
        AdminOperation::Unpause => ExecutionGuard::PauseEpoch(
            ControllerClient::new(env, &storage::get_controller(env))
                .get_pause_epoch()
                .unwrap_or_else(|| panic_with_error!(env, GenericError::PauseEpochMismatch)),
        ),
        AdminOperation::ConfigureAssetOracle(args) => {
            ExecutionGuard::OracleBand(args.key.clone(), env.ledger().sequence())
        }
        _ => return,
    };
    storage::set_execution_guard(env, operation_id, &guard);
}

/// Panics if the state `operation_id` was bound to at proposal has moved.
/// `target` is the contract the operation invokes. An `Unpause` also reverts
/// with `GenericError::ConflictingOperationPending` while an
/// `UpgradeController` is pending. An operation without a recorded guard
/// passes.
pub(crate) fn require_holds(env: &Env, operation_id: &BytesN<32>, target: &Address) {
    match storage::execution_guard(env, operation_id) {
        Some(ExecutionGuard::PauseEpoch(epoch)) => {
            assert_with_error!(
                env,
                !controller_upgrade_pending(env),
                GenericError::ConflictingOperationPending
            );
            assert_with_error!(
                env,
                ControllerClient::new(env, target).get_pause_epoch() == Some(epoch),
                GenericError::PauseEpochMismatch
            );
        }
        Some(ExecutionGuard::OracleBand(key, proposed_at)) => assert_with_error!(
            env,
            PriceAggregatorClient::new(env, target).sanity_band_narrowed_at(&key) < proposed_at,
            OracleError::SanityBandNarrowedAfterProposal
        ),
        None => {}
    }
}
