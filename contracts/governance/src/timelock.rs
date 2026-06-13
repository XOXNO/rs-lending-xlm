//! OpenZeppelin timelock embedded into the governance contract.
//!
//! Governance implements the OZ `Timelock` `#[contracttrait]`. The four
//! host-supplied methods (`schedule` / `execute` / `cancel` / `update_delay`)
//! renew the instance TTL, enforce auth + the matching role, then delegate to
//! the crate storage helper. The four read-only query methods come from the
//! trait defaults and are auto-exported by `#[contractimpl(contracttrait)]`.
//!
//! Auth model: PROPOSER gates `schedule`, EXECUTOR gates an explicit-executor
//! `execute` (open execution is allowed with `executor: None`), CANCELLER gates
//! `cancel`. `update_delay` is self-administered: it is callable only by the
//! governance contract itself, so a delay change must itself be scheduled and
//! pass the timelock delay.

use soroban_sdk::{contractimpl, Address, BytesN, Env, Symbol, Val, Vec};
use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, schedule_operation, set_min_delay, Operation,
    OperationState, Timelock,
};

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
use crate::storage::renew_governance_instance;
use crate::{Governance, GovernanceArgs, GovernanceClient};

#[contractimpl(contracttrait)]
impl Timelock for Governance {
    // Arity is fixed by the upstream OZ `Timelock` trait signature.
    #[allow(clippy::too_many_arguments)]
    fn schedule(
        env: &Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
        delay: u32,
        proposer: Address,
    ) -> BytesN<32> {
        renew_governance_instance(env);
        proposer.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, PROPOSER_ROLE), &proposer);
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        schedule_operation(env, &operation, delay)
    }

    fn execute(
        env: &Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
        executor: Option<Address>,
    ) -> Val {
        renew_governance_instance(env);
        if let Some(ref exec) = executor {
            exec.require_auth();
            access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);
        }
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        execute_operation(env, &operation)
    }

    fn cancel(env: &Env, operation_id: BytesN<32>, canceller: Address) {
        renew_governance_instance(env);
        canceller.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, CANCELLER_ROLE), &canceller);
        cancel_operation(env, &operation_id);
    }

    fn update_delay(env: &Env, new_delay: u32, operator: Address) {
        renew_governance_instance(env);
        // Self-administered: only the governance contract itself may change the
        // delay, so a delay change must be scheduled through the timelock and is
        // itself delayed. `operator` is carried for the trait signature; the
        // self-auth gate is the real authority check.
        let _ = operator;
        env.current_contract_address().require_auth();
        set_min_delay(env, new_delay);
    }
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, BytesN, Env, Symbol};
    use stellar_governance::timelock::OperationState;

    use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
    use crate::constants::TIMELOCK_MIN_DELAY_LEDGERS;
    use crate::{Governance, GovernanceClient};

    fn register(env: &Env, min_delay: u32) -> (Address, GovernanceClient<'_>) {
        let admin = Address::generate(env);
        let gov_id = env.register(Governance, (admin.clone(), min_delay));
        (admin, GovernanceClient::new(env, &gov_id))
    }

    #[test]
    fn constructor_grants_timelock_roles_to_admin() {
        let env = Env::default();
        let (admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        assert!(gov.has_role(&admin, &Symbol::new(&env, PROPOSER_ROLE)));
        assert!(gov.has_role(&admin, &Symbol::new(&env, EXECUTOR_ROLE)));
        assert!(gov.has_role(&admin, &Symbol::new(&env, CANCELLER_ROLE)));
    }

    #[test]
    fn constructor_sets_min_delay() {
        let env = Env::default();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        assert_eq!(gov.get_min_delay(), TIMELOCK_MIN_DELAY_LEDGERS);
    }

    #[test]
    fn get_operation_state_unknown_is_unset() {
        let env = Env::default();
        let (_admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);

        let unknown = BytesN::<32>::from_array(&env, &[7u8; 32]);
        assert_eq!(gov.get_operation_state(&unknown), OperationState::Unset);
    }

    // A scheduled self-targeted operation moves Unset -> Waiting and, after the
    // ledger crosses the ready ledger, -> Ready. Proves the host `schedule`
    // wiring and the default `get_operation_state`/`hash_operation` exports.
    #[test]
    fn schedule_transitions_unset_waiting_ready() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, gov) = register(&env, delay);

        let target = gov.address.clone();
        let function = Symbol::new(&env, "noop");
        let args = soroban_sdk::Vec::new(&env);
        let predecessor = BytesN::<32>::from_array(&env, &[0u8; 32]);
        let salt = BytesN::<32>::from_array(&env, &[1u8; 32]);

        let id = gov.schedule(
            &target,
            &function,
            &args,
            &predecessor,
            &salt,
            &delay,
            &admin,
        );

        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

        env.ledger().with_mut(|l| l.sequence_number += delay);
        assert_eq!(gov.get_operation_state(&id), OperationState::Ready);
    }
}
