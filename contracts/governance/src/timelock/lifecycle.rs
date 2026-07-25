//! Timelock lifecycle: propose, execute, and cancel a scheduled
//! `AdminOperation`. Every entrypoint here enforces the delay.

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
    /// Schedules an `AdminOperation` and returns its operation id. `PROPOSER`-gated.
    /// Sensitive floor: upgrades, ownership transfers, `SetPriceAggregator`. Other
    /// ops use min delay. `TransferGovOwnership` requires the owner as proposer;
    /// `RevokeGovRole` may not target the proposer or the owner.
    ///
    /// # Errors
    /// * `NotAuthorized` — revoke self/owner, or non-owner proposes ownership transfer.
    /// * `PoolNotInitialized` / `AggregatorNotSet` — target not wired yet.
    /// * Via `resolve_op`: `InvalidWasmHash`, `InvalidTimelockDelay`, `InvalidRole`,
    ///   `InvalidAggregator`, `NotSmartContract`, `InvalidPositionLimits`,
    ///   `InvalidBorrowParams`, `WrongToken`, `InvalidAsset`, `BadLastTolerance`,
    ///   `InvalidExchangeSrc`, and live oracle-probe reverts.
    /// * Access-control / OZ timelock reject unknown proposer or duplicate schedule.
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
            // A proposer may revoke anyone's role except its own; the owner's
            // roles are never revocable. The owner check is re-enforced at apply.
            crate::op::AdminOperation::RevokeGovRole(args) => {
                assert_with_error!(&env, args.account != proposer, GenericError::NotAuthorized);
                assert_with_error!(
                    &env,
                    args.account != access::owner_or_panic(&env),
                    GenericError::NotAuthorized
                );
            }
            // Only the owner may initiate an ownership transfer; any canceller
            // can still veto it during the timelock.
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
        // Record the target so `cancel` can enforce the self-veto guard.
        if let crate::op::AdminOperation::RevokeGovRole(args) = &op {
            storage::mark_role_revocation_target(&env, &operation_id, &args.account);
        }
        operation_id
    }

    /// Executes a ready non-self timelock operation and returns its result.
    /// `Some(executor)` requires `EXECUTOR` auth; `None` leaves execution open.
    /// Self-ops must use `execute_self`.
    ///
    /// # Errors
    /// * `InternalError` — `target` is this governance contract.
    /// * `TimelockOperationExpired` — past grace window.
    /// * OZ timelock rejects not-scheduled / not-ready; `EXECUTOR` gate when set.
    ///
    /// # Events
    /// * OZ timelock execute event; target emits its own.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready operation.
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

    /// Applies a ready governance-self op inline (upgrade, delay, roles,
    /// ownership, `SetPriceAggregator`). `Some(executor)` requires `EXECUTOR`;
    /// `None` leaves execution open.
    ///
    /// # Errors
    /// * `InternalError` — `op` does not target this contract.
    /// * `TimelockOperationExpired` — past grace window.
    /// * `InvalidTimelockDelay`, `InvalidRole`, `OwnerNotSet`, `InvalidAggregator`
    ///   on self-apply; OZ not-scheduled / not-ready.
    ///
    /// # Events
    /// * OZ timelock execute event plus role / ownership / upgrade events.
    ///
    /// # Security Warning
    /// * With `executor` = `None` any caller may execute a ready self-operation.
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
        // Self-target execute is inline; Soroban blocks self-reentry.
        let operation_id = prepare_execute(&env, executor.as_ref(), &operation);
        set_execute_operation(&env, &operation);
        apply_self_op(&env, &op);
        finish_execute(&env, &operation_id);
    }

    /// Cancels a pending timelock operation. `CANCELLER`-gated.
    /// Recovery-tier ops and self-targeted role revocations are not cancellable.
    ///
    /// # Errors
    /// * `OperationNotCancellable` — Recovery op, or revoke of `canceller`'s own role.
    /// * Access-control / OZ timelock reject unknown canceller or not-pending.
    ///
    /// # Events
    /// * OZ timelock cancel event.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        storage::renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        // Recovery-tier operations are non-vetoable — they exist precisely to
        // override a captured canceller council.
        assert_with_error!(
            &env,
            !storage::is_recovery_op(&env, &operation_id),
            GenericError::OperationNotCancellable
        );
        // A role revocation cannot be vetoed by its own target — no one blocks
        // their own removal. Every other pending operation, including a
        // revocation of another canceller, stays vetoable, so the independent
        // cancellers remain a real check on a rogue proposer (or owner). A
        // colluding-canceller deadlock is broken by the non-vetoable Recovery
        // tier (`propose_canceller_reset`), not by suspending the veto here.
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
