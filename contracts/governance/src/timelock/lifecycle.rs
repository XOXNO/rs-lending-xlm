//! Proposal, execution, and cancellation of timelocked governance operations:
//! scheduling an operation after role and argument checks, executing it once its
//! delay has elapsed (either against an external target or against this contract
//! itself), and cancelling a pending operation.

use common::errors::GenericError;

use soroban_sdk::{assert_with_error, Address, BytesN, Env, Symbol, Val, Vec};

use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, schedule_operation, set_execute_operation, Operation,
};

use crate::access::{self, CANCELLER_ROLE, PROPOSER_ROLE};
use crate::op::apply_self_op;
use crate::storage;
use crate::timelock::*;

/// Schedules `op` for later execution and returns its operation id; requires the
/// caller to hold `PROPOSER_ROLE`. For a `RevokeGovRole` operation, rejects the
/// proposer targeting themselves or the owner and records the target account so
/// it cannot later cancel its own revocation; for `TransferGovOwnership`, requires
/// the proposer to be the current owner. The operation's delay is derived from the
/// resolved operation's delay tier.
pub(crate) fn propose(
    env: &Env,
    proposer: &Address,
    op: &crate::op::AdminOperation,
    salt: BytesN<32>,
) -> BytesN<32> {
    begin_immediate(env, proposer, PROPOSER_ROLE);
    match op {
        crate::op::AdminOperation::RevokeGovRole(args) => {
            assert_with_error!(env, &args.account != proposer, GenericError::NotAuthorized);
            assert_with_error!(
                env,
                args.account != access::owner_or_panic(env),
                GenericError::NotAuthorized
            );
        }
        crate::op::AdminOperation::TransferGovOwnership(_) => {
            assert_with_error!(
                env,
                proposer == &access::owner_or_panic(env),
                GenericError::NotAuthorized
            );
        }
        _ => {}
    }
    let (operation, delay_tier) = operation_for_admin_op(env, op, salt);
    let delay = operation_delay(env, delay_tier);
    let operation_id = schedule_operation(env, &operation, delay);
    if let crate::op::AdminOperation::RevokeGovRole(args) = op {
        storage::mark_role_revocation_target(env, &operation_id, &args.account);
    }
    operation_id
}

/// Executes a scheduled operation against `target` once its delay has elapsed and
/// it has not expired, and returns the invocation's result. Rejects operations
/// that target this contract itself (use `execute_self` for those). Clears the
/// operation's scheduled state on completion.
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
    let operation_id = prepare_execute(env, executor.as_ref(), &operation);
    let result = execute_operation(env, &operation);
    finish_execute(env, &operation_id);
    result
}

/// Executes a scheduled admin operation that targets this contract itself, once
/// its delay has elapsed and it has not expired. Rejects operations resolved to a
/// different target. Clears the operation's scheduled state on completion.
pub(crate) fn execute_self(
    env: &Env,
    executor: Option<Address>,
    op: &crate::op::AdminOperation,
    salt: BytesN<32>,
) {
    let (operation, _) = operation_for_admin_op(env, op, salt);
    assert_with_error!(
        env,
        operation.target == env.current_contract_address(),
        GenericError::InternalError
    );
    let operation_id = prepare_execute(env, executor.as_ref(), &operation);
    set_execute_operation(env, &operation);
    apply_self_op(env, op);
    finish_execute(env, &operation_id);
}

/// Cancels a pending operation; requires the caller to hold `CANCELLER_ROLE`.
/// Rejects cancelling a recovery operation or a role-revocation operation whose
/// target account is the canceller. Clears the operation's sidecar state on
/// success.
pub(crate) fn cancel(env: &Env, canceller: &Address, operation_id: &BytesN<32>) {
    storage::renew_governance_instance(env);
    canceller.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, CANCELLER_ROLE), canceller);
    assert_with_error!(
        env,
        !storage::is_recovery_op(env, operation_id),
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
