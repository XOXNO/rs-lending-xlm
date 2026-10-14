//! Proposal, execution, and cancellation of timelocked governance operations:
//! scheduling an operation after role and argument checks, executing it once its
//! delay has elapsed (either against an external target or against this contract
//! itself), and cancelling a pending operation.

use common::errors::GenericError;
use common::ttl::renew_instance;

use soroban_sdk::{assert_with_error, Address, BytesN, Env, IntoVal, Symbol, Val, Vec};

use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, schedule_operation, set_execute_operation, Operation,
};

use crate::access::{self, CANCELLER_ROLE, PROPOSER_ROLE};
use crate::op::{apply_self_op, requires_owner_proposer, AdminOperation};
use crate::storage;
use crate::timelock::*;

/// Schedules `op` for later execution and returns its operation id; requires the
/// caller to hold `PROPOSER_ROLE`. An expired operation with the same id is
/// cleared first.
///
/// `RevokeGovRole` rejects a target that is the proposer or the owner, and records
/// the target so it cannot cancel its own revocation. Ownership transfers, code
/// upgrades, controller migration, the timelock minimum delay, price aggregator
/// and oracle configuration, the swap aggregator, Blend pool approval, the
/// revenue accumulator, role grants, the spoke liquidation curve, forced
/// socialization, position-manager activation and a listing edit that lowers
/// the liquidation threshold, raises the bonus or lowers the fees of the live
/// listing also require the proposer to be the owner. These checks fail with
/// `GenericError::NotAuthorized`. The operation's guard records the
/// governance state it is bound to (the proposer's epoch for every proposal,
/// which a handover away from the owner advances for the owner's own, and
/// the live terms a non-owner listing edit kept), and execution rejects it
/// once that state has moved. A proposed `UpgradeController` counts as
/// pending until it leaves the queue. The delay comes from the operation's
/// delay tier.
pub(crate) fn propose(
    env: &Env,
    proposer: &Address,
    op: &AdminOperation,
    salt: BytesN<32>,
) -> BytesN<32> {
    begin_immediate(env, proposer, PROPOSER_ROLE);
    if requires_owner_proposer(op) {
        assert_with_error!(
            env,
            proposer == &access::owner_or_panic(env),
            GenericError::NotAuthorized
        );
    }
    if let AdminOperation::RevokeGovRole(args) = op {
        assert_with_error!(env, &args.account != proposer, GenericError::NotAuthorized);
        assert_with_error!(
            env,
            args.account != access::owner_or_panic(env),
            GenericError::NotAuthorized
        );
    }
    let (operation, delay_tier) = operation_for_admin_op(env, op, salt);
    let guard = guard::for_op(env, proposer, op, &operation);
    if let AdminOperation::EditAssetInSpoke(args) = op {
        guard::require_terms_kept(env, guard.listing_terms, args);
    }
    let delay = operation_delay(env, delay_tier);
    clear_expired_operation(env, &operation);
    let operation_id = schedule_operation(env, &operation, delay);
    if let AdminOperation::RevokeGovRole(args) = op {
        storage::mark_role_revocation_target(env, &operation_id, &args.account);
    }
    if matches!(op, AdminOperation::UpgradeController(_)) {
        storage::mark_pending_controller_upgrade(env, &operation_id);
    }
    storage::set_operation_guard(env, &operation_id, &guard);
    operation_id
}

/// Executes a scheduled operation against `target` once its delay has elapsed and
/// it has not expired, and returns the invocation's result. Rejects operations
/// that target this contract itself (use `execute_self` for those). Clears the
/// operation's scheduled state on completion. An upgrade advances its
/// target's upgrade epoch; a controller upgrade also counts as an emergency
/// action and advances the emergency epoch; a controller nomination advances
/// the controller nomination epoch. A controller nomination cancellation
/// (`TransferCtrlOwnership` with `live_until_ledger` 0) proposed before a
/// later controller nomination executed completes without calling the
/// controller.
pub(crate) fn execute(
    env: &Env,
    executor: Option<Address>,
    target: Address,
    function: Symbol,
    args: Vec<Val>,
    predecessor: BytesN<32>,
    salt: BytesN<32>,
) -> Val {
    assert_with_error!(
        env,
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
    let (operation_id, guard) = prepare_execute(env, executor.as_ref(), &operation);
    let result = if guard::is_stale_controller_cancel(env, &operation, &guard) {
        set_execute_operation(env, &operation);
        ().into_val(env)
    } else {
        execute_operation(env, &operation)
    };
    guard::after_execute(env, &operation);
    finish_execute(env, &operation_id);
    result
}

/// Executes a scheduled admin operation that targets this contract itself, once
/// its delay has elapsed and it has not expired. Rejects operations resolved to a
/// different target. Clears the operation's scheduled state on completion. A
/// governance upgrade advances the governance upgrade epoch.
///
/// A nomination cancellation (`TransferGovOwnership` with `live_until_ledger`
/// 0) recorded against an earlier nomination, or whose account is no longer
/// pending, completes without changing the pending owner.
pub(crate) fn execute_self(
    env: &Env,
    executor: Option<Address>,
    op: &AdminOperation,
    salt: BytesN<32>,
) {
    let (operation, _) = operation_for_admin_op(env, op, salt);
    assert_with_error!(
        env,
        operation.target == env.current_contract_address(),
        GenericError::InternalError
    );
    let (operation_id, guard) = prepare_execute(env, executor.as_ref(), &operation);
    set_execute_operation(env, &operation);
    if targets_live_nomination(env, op, guard.kind_epoch) {
        apply_self_op(env, op);
    }
    guard::after_execute(env, &operation);
    finish_execute(env, &operation_id);
}

/// Returns `false` only for a nomination cancellation whose recorded
/// nomination is no longer the pending one.
fn targets_live_nomination(env: &Env, op: &AdminOperation, nomination_epoch: Option<u64>) -> bool {
    match (op, nomination_epoch) {
        (AdminOperation::TransferGovOwnership(args), Some(epoch)) => {
            access::nomination_cancel_is_current(env, &args.new_owner, epoch)
        }
        _ => true,
    }
}

/// Cancels a pending operation; requires the caller to hold `CANCELLER_ROLE`.
/// Rejects cancelling a recovery operation that can still execute, or a
/// role-revocation operation whose target account is the canceller. A
/// recovery operation voided by a later ownership handover can never execute,
/// so any canceller may clear it and free its id. Clears the operation's
/// sidecar state on success.
pub(crate) fn cancel(env: &Env, canceller: &Address, operation_id: &BytesN<32>) {
    renew_instance(env);
    canceller.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, CANCELLER_ROLE), canceller);
    assert_with_error!(
        env,
        !storage::is_recovery_op(env, operation_id)
            || guard::canceller_reset_is_stale(env, operation_id),
        GenericError::OperationNotCancellable
    );
    if let Some(target) = storage::role_revocation_target(env, operation_id) {
        assert_with_error!(
            env,
            &target != canceller,
            GenericError::OperationNotCancellable
        );
    }
    cancel_operation(env, operation_id);
    storage::clear_operation_sidecars(env, operation_id);
}
