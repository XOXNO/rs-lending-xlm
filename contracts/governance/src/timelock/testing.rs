//! Test-only immediate executor. Excluded from production WASM.

use soroban_sdk::{contractimpl, Address, BytesN, Env, IntoVal, Symbol, Val};

use crate::op::apply_self_op;
use crate::timelock::*;
use crate::{storage, Governance, GovernanceArgs, GovernanceClient};

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
    /// Runs an `AdminOperation` with no delay. Oracle config ops require
    /// `ORACLE`; all others require the owner.
    pub fn execute_immediate(env: Env, caller: Address, op: crate::op::AdminOperation) -> Val {
        storage::renew_governance_instance(&env);
        caller.require_auth();
        match &op {
            crate::op::AdminOperation::ConfigureAssetOracle(_)
            | crate::op::AdminOperation::EditOracleTolerance(_) => {
                stellar_access::access_control::ensure_role(
                    &env,
                    &Symbol::new(&env, crate::access::ORACLE_ROLE),
                    &caller,
                );
            }
            _ => {
                let owner = stellar_access::ownable::get_owner(&env)
                    .unwrap_or_else(|| panic!("Owner not set"));
                assert_eq!(caller, owner, "not owner");
            }
        }
        let (operation, _) =
            operation_for_admin_op(&env, &op, BytesN::from_array(&env, &[0u8; 32]));
        if operation.target == env.current_contract_address() {
            apply_self_op(&env, &op);
            ().into_val(&env)
        } else {
            env.invoke_contract(&operation.target, &operation.function, operation.args)
        }
    }
}
