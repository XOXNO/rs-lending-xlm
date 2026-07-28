//! Delayed `AdminOperation` lifecycle: propose, execute, cancel.

use common::errors::GenericError;

use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env, Symbol, Val, Vec};

use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, schedule_operation, set_execute_operation, Operation,
};

use crate::access::{self, CANCELLER_ROLE, PROPOSER_ROLE};
use crate::op::apply_self_op;
use crate::timelock::*;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

#[contractimpl]
impl Governance {
    /// Schedules an `AdminOperation` and returns its operation id. `PROPOSER`
    /// only. Sensitive floor applies to upgrades, ownership transfers, and
    /// `SetPriceAggregator`. `TransferGovOwnership` requires the owner as
    /// proposer. `RevokeGovRole` may not target the proposer or the owner.
    ///
    /// # Errors
    /// * [`GenericError::NotAuthorized`] — self/owner revoke, or non-owner
    ///   proposes ownership transfer.
    /// * [`GenericError::PoolNotInitialized`] / [`GenericError::AggregatorNotSet`]
    ///   — target not wired.
    /// * Via `resolve_op`: invalid wasm, delay, role, aggregator, limits, asset,
    ///   tolerance, and live oracle-probe reverts.
    /// * Access-control / OZ timelock reject unknown proposer or duplicate id.
    ///
    /// # Events
    /// * OZ timelock schedule event.
    pub fn propose(
        env: Env,
        proposer: Address,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_immediate(&env, &proposer, PROPOSER_ROLE);
        match &op {
            crate::op::AdminOperation::RevokeGovRole(args) => {
                assert_with_error!(&env, args.account != proposer, GenericError::NotAuthorized);
                assert_with_error!(
                    &env,
                    args.account != access::owner_or_panic(&env),
                    GenericError::NotAuthorized
                );
            }
            crate::op::AdminOperation::TransferGovOwnership(_) => {
                assert_with_error!(
                    &env,
                    proposer == access::owner_or_panic(&env),
                    GenericError::NotAuthorized
                );
            }
            _ => {}
        }
        let (operation, delay_tier) = operation_for_admin_op(&env, &op, salt);
        let delay = operation_delay(&env, delay_tier);
        let operation_id = schedule_operation(&env, &operation, delay);
        if let crate::op::AdminOperation::RevokeGovRole(args) = &op {
            storage::mark_role_revocation_target(&env, &operation_id, &args.account);
        }
        operation_id
    }

    /// Executes a ready non-self operation and returns its result.
    /// `Some(executor)` requires `EXECUTOR` auth; `None` is open execution.
    /// Self-ops use `execute_self`.
    ///
    /// # Errors
    /// * [`GenericError::InternalError`] — `target` is this contract.
    /// * [`GenericError::TimelockOperationExpired`] — past grace.
    /// * OZ not-scheduled / not-ready; `EXECUTOR` gate when set.
    ///
    /// # Events
    /// * OZ execute event; target emits its own.
    ///
    /// # Security Warning
    /// * With `executor` = `None`, any caller may execute a ready operation.
    pub fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val {
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
        let operation_id = prepare_execute(&env, executor.as_ref(), &operation);
        let result = execute_operation(&env, &operation);
        finish_execute(&env, &operation_id);
        result
    }

    /// Applies a ready governance-self op (upgrade, delay, roles, ownership,
    /// `SetPriceAggregator`). Inline apply — Soroban blocks self-reentry.
    /// `Some(executor)` requires `EXECUTOR`; `None` is open execution.
    ///
    /// # Errors
    /// * [`GenericError::InternalError`] — `op` is not self-target.
    /// * [`GenericError::TimelockOperationExpired`] — past grace.
    /// * Self-apply: invalid delay, role, owner, aggregator; owner revoke;
    ///   last-proposer remove. OZ not-scheduled / not-ready.
    ///
    /// # Events
    /// * OZ execute event; role / ownership / upgrade events as applicable.
    ///
    /// # Security Warning
    /// * With `executor` = `None`, any caller may execute a ready self-op.
    pub fn execute_self(
        env: Env,
        executor: Option<Address>,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) {
        let (operation, _) = operation_for_admin_op(&env, &op, salt);
        assert_with_error!(
            env,
            operation.target == env.current_contract_address(),
            GenericError::InternalError
        );
        let operation_id = prepare_execute(&env, executor.as_ref(), &operation);
        set_execute_operation(&env, &operation);
        apply_self_op(&env, &op);
        finish_execute(&env, &operation_id);
    }

    /// Cancels a pending operation. `CANCELLER` only.
    ///
    /// Not cancellable: Recovery-tier ops, and role revocations whose target
    /// is the canceller (self-veto).
    ///
    /// # Errors
    /// * [`GenericError::OperationNotCancellable`] — Recovery mark or self-veto.
    /// * Access-control / OZ reject unknown canceller or not-pending id.
    ///
    /// # Events
    /// * OZ cancel event.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        storage::renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        assert_with_error!(
            &env,
            !storage::is_recovery_op(&env, &operation_id),
            GenericError::OperationNotCancellable
        );
        if let Some(target) = storage::role_revocation_target(&env, &operation_id) {
            assert_with_error!(
                &env,
                target != canceller,
                GenericError::OperationNotCancellable
            );
        }
        cancel_operation(&env, &operation_id);
        storage::clear_operation_sidecars(&env, &operation_id);
    }
}
