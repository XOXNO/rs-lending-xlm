//! Timelocked governance-self admin via inline dispatch.
//!
//! Soroban prohibits `invoke_contract` self-reentry. Scheduled self operations
//! use OZ `set_execute_operation` for the timelock state machine, then apply the
//! mutation inline in the same frame.

use soroban_sdk::{contractimpl, Address, BytesN, Env};
use stellar_governance::timelock::{set_execute_operation, Operation};

use crate::storage::renew_governance_instance;
use crate::timelock::{authorize_executor, require_operation_not_expired};
use crate::{Governance, GovernanceArgs, GovernanceClient};

fn begin_self_execute(env: &Env, executor: Option<Address>, operation: &Operation) {
    renew_governance_instance(env);
    authorize_executor(env, executor.as_ref());
    require_operation_not_expired(env, operation);
    set_execute_operation(env, operation);
}

#[contractimpl]
impl Governance {
    pub fn execute_self(
        env: Env,
        executor: Option<Address>,
        op: crate::op::AdminOperation,
        salt: BytesN<32>,
    ) {
        let (target, function, args, _) = crate::op::resolve_op(&env, &op);
        assert!(target == env.current_contract_address());
        let operation = Operation {
            target,
            function,
            args,
            predecessor: BytesN::from_array(&env, &[0u8; 32]),
            salt,
        };
        begin_self_execute(&env, executor, &operation);
        crate::op::apply_self_op(&env, &op);
    }
}

#[cfg(test)]
#[path = "../tests/self_timelock.rs"]
mod tests;
