//! Timelocked governance-self admin via inline dispatch.
//!
//! Soroban prohibits `invoke_contract` self-reentry, so scheduled self ops use
//! OZ `set_execute_operation` for the state machine and apply the mutation
//! inline in the same frame. Typed `propose_*` / `execute_*` entrypoints mirror
//! the controller-targeted flow in `forward.rs` / `timelock.rs`.

use soroban_sdk::{contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val};
use stellar_access::access_control;
use stellar_governance::timelock::{
    get_min_delay, schedule_operation, set_execute_operation, Operation,
};

use crate::access::{
    apply_grant_role, apply_revoke_role, apply_transfer_ownership, apply_upgrade,
    require_known_governance_role, PROPOSER_ROLE,
};
use crate::storage::renew_governance_instance;
use crate::timelock::{
    apply_update_delay, authorize_executor, require_operation_not_expired, validate_delay_update,
};
use crate::{Governance, GovernanceArgs, GovernanceClient};

fn begin_proposal(env: &Env, proposer: &Address) {
    renew_governance_instance(env);
    proposer.require_auth();
    access_control::ensure_role(env, &Symbol::new(env, PROPOSER_ROLE), proposer);
}

fn schedule_self_op(
    env: &Env,
    function: &str,
    args: soroban_sdk::Vec<Val>,
    salt: BytesN<32>,
) -> BytesN<32> {
    let operation = Operation {
        target: env.current_contract_address(),
        function: Symbol::new(env, function),
        args,
        predecessor: BytesN::from_array(env, &[0u8; 32]),
        salt,
    };
    schedule_operation(env, &operation, get_min_delay(env))
}

fn self_operation(
    env: &Env,
    function: &str,
    args: soroban_sdk::Vec<Val>,
    salt: &BytesN<32>,
) -> Operation {
    Operation {
        target: env.current_contract_address(),
        function: Symbol::new(env, function),
        args,
        predecessor: BytesN::from_array(env, &[0u8; 32]),
        salt: salt.clone(),
    }
}

fn begin_self_execute(env: &Env, executor: Option<Address>, operation: &Operation) {
    renew_governance_instance(env);
    authorize_executor(env, executor.as_ref());
    require_operation_not_expired(env, operation);
    set_execute_operation(env, operation);
}

#[contractimpl]
impl Governance {
    pub fn propose_governance_upgrade(
        env: Env,
        proposer: Address,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_self_op(
            &env,
            "upgrade",
            vec![&env, new_wasm_hash.into_val(&env)],
            salt,
        )
    }

    pub fn execute_governance_upgrade(
        env: Env,
        executor: Option<Address>,
        new_wasm_hash: BytesN<32>,
        salt: BytesN<32>,
    ) {
        let operation = self_operation(
            &env,
            "upgrade",
            vec![&env, new_wasm_hash.into_val(&env)],
            &salt,
        );
        begin_self_execute(&env, executor, &operation);
        apply_upgrade(&env, &new_wasm_hash);
    }

    pub fn propose_update_delay(
        env: Env,
        proposer: Address,
        new_delay: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        validate_delay_update(&env, new_delay);
        schedule_self_op(
            &env,
            "update_delay",
            vec![&env, new_delay.into_val(&env)],
            salt,
        )
    }

    pub fn execute_update_delay(
        env: Env,
        executor: Option<Address>,
        new_delay: u32,
        salt: BytesN<32>,
    ) {
        let operation = self_operation(
            &env,
            "update_delay",
            vec![&env, new_delay.into_val(&env)],
            &salt,
        );
        begin_self_execute(&env, executor, &operation);
        apply_update_delay(&env, new_delay);
    }

    pub fn propose_grant_governance_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        require_known_governance_role(&env, &role);
        schedule_self_op(
            &env,
            "grant_role",
            vec![&env, account.into_val(&env), role.into_val(&env)],
            salt,
        )
    }

    pub fn execute_grant_governance_role(
        env: Env,
        executor: Option<Address>,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) {
        let operation = self_operation(
            &env,
            "grant_role",
            vec![
                &env,
                account.clone().into_val(&env),
                role.clone().into_val(&env),
            ],
            &salt,
        );
        begin_self_execute(&env, executor, &operation);
        apply_grant_role(&env, &account, &role);
    }

    pub fn propose_revoke_governance_role(
        env: Env,
        proposer: Address,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        require_known_governance_role(&env, &role);
        schedule_self_op(
            &env,
            "revoke_role",
            vec![&env, account.into_val(&env), role.into_val(&env)],
            salt,
        )
    }

    pub fn execute_revoke_governance_role(
        env: Env,
        executor: Option<Address>,
        account: Address,
        role: Symbol,
        salt: BytesN<32>,
    ) {
        let operation = self_operation(
            &env,
            "revoke_role",
            vec![
                &env,
                account.clone().into_val(&env),
                role.clone().into_val(&env),
            ],
            &salt,
        );
        begin_self_execute(&env, executor, &operation);
        apply_revoke_role(&env, &account, &role);
    }

    pub fn propose_transfer_gov_own(
        env: Env,
        proposer: Address,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        begin_proposal(&env, &proposer);
        schedule_self_op(
            &env,
            "transfer_ownership",
            vec![
                &env,
                new_owner.into_val(&env),
                live_until_ledger.into_val(&env),
            ],
            salt,
        )
    }

    pub fn execute_transfer_gov_own(
        env: Env,
        executor: Option<Address>,
        new_owner: Address,
        live_until_ledger: u32,
        salt: BytesN<32>,
    ) {
        let operation = self_operation(
            &env,
            "transfer_ownership",
            vec![
                &env,
                new_owner.clone().into_val(&env),
                live_until_ledger.into_val(&env),
            ],
            &salt,
        );
        begin_self_execute(&env, executor, &operation);
        apply_transfer_ownership(&env, &new_owner, live_until_ledger);
    }
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, BytesN, Env, Symbol};
    use stellar_governance::timelock::OperationState;

    use crate::access::EXECUTOR_ROLE;
    use crate::{Governance, GovernanceClient};

    const ZERO_SALT: [u8; 32] = [0u8; 32];

    fn register(env: &Env, min_delay: u32) -> (Address, GovernanceClient<'_>) {
        let admin = Address::generate(env);
        let gov_id = env.register(Governance, (admin.clone(), min_delay));
        (admin, GovernanceClient::new(env, &gov_id))
    }

    #[test]
    fn propose_update_delay_schedules_waiting_operation() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, gov) = register(&env, delay);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_update_delay(&admin, &15u32, &salt);
        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #39)")]
    fn propose_update_delay_rejects_shortening() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_update_delay(&admin, &5u32, &salt);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #39)")]
    fn propose_update_delay_rejects_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_update_delay(&admin, &0u32, &salt);
    }

    #[test]
    fn execute_update_delay_applies_after_delay() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, gov) = register(&env, delay);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_update_delay(&admin, &15u32, &salt);
        env.ledger().with_mut(|l| l.sequence_number += delay);
        assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

        gov.execute_update_delay(&Some(admin.clone()), &15u32, &salt);
        assert_eq!(gov.get_min_delay(), 15u32);
        assert_eq!(gov.get_operation_state(&id), OperationState::Done);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #41)")]
    fn propose_grant_governance_role_rejects_unknown_role() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_grant_governance_role(&admin, &admin, &Symbol::new(&env, "KEEPER"), &salt);
    }

    #[test]
    #[should_panic]
    fn non_proposer_cannot_propose_governance_upgrade() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, gov) = register(&env, 10);
        let stranger = Address::generate(&env);
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_governance_upgrade(&stranger, &BytesN::from_array(&env, &ZERO_SALT), &salt);
    }

    #[test]
    fn execute_grant_governance_role_after_delay() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, gov) = register(&env, delay);
        let grantee = Address::generate(&env);
        let role = Symbol::new(&env, EXECUTOR_ROLE);
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);

        gov.propose_grant_governance_role(&admin, &grantee, &role, &salt);
        env.ledger().with_mut(|l| l.sequence_number += delay);
        gov.execute_grant_governance_role(&Some(admin.clone()), &grantee, &role, &salt);
        assert!(gov.has_role(&grantee, &role));
    }
}
