//! Timelocked governance-self admin via inline dispatch.
//!
//! Soroban prohibits `invoke_contract` self-reentry. Scheduled self operations
//! use OZ `set_execute_operation` for the timelock state machine, then apply the
//! mutation inline in the same frame. Typed `propose_*` and `execute_*`
//! entrypoints match the controller-targeted flow.

use soroban_sdk::{contractimpl, vec, Address, BytesN, Env, IntoVal, Symbol, Val};
use stellar_access::access_control;
use stellar_governance::timelock::{schedule_operation, set_execute_operation, Operation};

use crate::access::{
    apply_grant_role, apply_revoke_role, apply_transfer_ownership, apply_upgrade,
    require_known_governance_role, PROPOSER_ROLE,
};
use crate::validate;
use crate::storage::renew_governance_instance;
use crate::timelock::{
    apply_update_delay, authorize_executor, operation_delay, require_operation_not_expired,
    validate_delay_update, DelayTier,
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
    delay: u32,
) -> BytesN<32> {
    let operation = Operation {
        target: env.current_contract_address(),
        function: Symbol::new(env, function),
        args,
        predecessor: BytesN::from_array(env, &[0u8; 32]),
        salt,
    };
    schedule_operation(env, &operation, delay)
}

macro_rules! self_delay_tier {
    () => {
        DelayTier::Standard
    };
    (sensitive) => {
        DelayTier::Sensitive
    };
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

/// Per-argument marshalling for the execute path. `byref` arguments are reused
/// by the `apply:` expression after the `Operation` is built, so they are cloned
/// into the replayed args; `byval` arguments are not reused and are marshalled
/// directly. The propose path always marshals directly because nothing reuses
/// the arguments after scheduling.
macro_rules! exec_arg {
    (byref $arg:ident, $env:ident) => {
        $arg.clone().into_val(&$env)
    };
    (byval $arg:ident, $env:ident) => {
        $arg.into_val(&$env)
    };
}

/// Declares a governance-self timelock operation as a `propose_*` / `execute_*`
/// pair from a single row. The OZ timelock keys an operation by
/// `(target, function, args, ...)`, so propose and execute must agree on the
/// function symbol and replayed args; sourcing both from one declaration makes
/// that agreement structural rather than a hand-maintained invariant.
///
/// Each argument is tagged `byref` (cloned in the execute replay because the
/// `apply:` expression borrows it afterwards) or `byval` (marshalled directly).
macro_rules! self_timelock_ops {
    ( $env:ident ; $(
        op($propose:ident, $execute:ident, $func:literal)
        ( $( $kind:tt $arg:ident : [ $( $argty:tt )+ ] ),* $(,)? )
        $( delay: $delay_tier:ident ; )?
        $( validate: $validate:expr ; )?
        apply: $apply:expr ;
    )* ) => {
        #[contractimpl]
        impl Governance {
            $(
                pub fn $propose(
                    $env: Env,
                    proposer: Address,
                    $( $arg: $( $argty )+, )*
                    salt: BytesN<32>,
                ) -> BytesN<32> {
                    begin_proposal(&$env, &proposer);
                    $( $validate; )?
                    let delay = operation_delay(
                        &$env,
                        self_delay_tier!($($delay_tier)?),
                    );
                    schedule_self_op(
                        &$env,
                        $func,
                        vec![&$env, $( $arg.into_val(&$env) ),*],
                        salt,
                        delay,
                    )
                }

                pub fn $execute(
                    $env: Env,
                    executor: Option<Address>,
                    $( $arg: $( $argty )+, )*
                    salt: BytesN<32>,
                ) {
                    let operation = self_operation(
                        &$env,
                        $func,
                        vec![&$env, $( exec_arg!($kind $arg, $env) ),*],
                        &salt,
                    );
                    begin_self_execute(&$env, executor, &operation);
                    $apply;
                }
            )*
        }
    };
}

// `env;` binds the identifier that the `validate:` / `apply:` expressions below
// reference, sharing its hygiene context with the generated `env` parameter.
self_timelock_ops! {
    env;
    op(propose_governance_upgrade, execute_governance_upgrade, "upgrade")
        (byval new_wasm_hash: [BytesN<32>])
        delay: sensitive ;
        validate: validate::require_nonzero_wasm_hash(&env, &new_wasm_hash);
        apply: apply_upgrade(&env, &new_wasm_hash);

    op(propose_update_delay, execute_update_delay, "update_delay")
        (byval new_delay: [u32])
        validate: validate_delay_update(&env, new_delay);
        apply: apply_update_delay(&env, new_delay);

    op(propose_grant_governance_role, execute_grant_governance_role, "grant_role")
        (byref account: [Address], byref role: [Symbol])
        validate: require_known_governance_role(&env, &role);
        apply: apply_grant_role(&env, &account, &role);

    op(propose_revoke_governance_role, execute_revoke_governance_role, "revoke_role")
        (byref account: [Address], byref role: [Symbol])
        validate: require_known_governance_role(&env, &role);
        apply: apply_revoke_role(&env, &account, &role);

    op(propose_transfer_gov_own, execute_transfer_gov_own, "transfer_ownership")
        (byref new_owner: [Address], byval live_until_ledger: [u32])
        delay: sensitive ;
        apply: apply_transfer_ownership(&env, &new_owner, live_until_ledger);
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, BytesN, Env, Symbol};
    use stellar_governance::timelock::OperationState;

    use crate::access::EXECUTOR_ROLE;
    use crate::constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS;
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
    #[should_panic(expected = "Error(Contract, #39)")]
    fn propose_update_delay_rejects_above_max_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let over_max = crate::constants::TIMELOCK_MAX_DELAY_LEDGERS + 1;
        gov.propose_update_delay(&admin, &over_max, &salt);
    }

    #[test]
    fn propose_governance_upgrade_uses_sensitive_delay() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let hash = BytesN::from_array(&env, &[9u8; 32]);

        let current = env.ledger().sequence();
        let id = gov.propose_governance_upgrade(&admin, &hash, &salt);

        assert_eq!(
            gov.get_operation_ledger(&id),
            current + TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS
        );
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
    #[should_panic(expected = "Error(Contract, #10)")]
    fn propose_governance_upgrade_rejects_zero_hash() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, gov) = register(&env, 10);
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_governance_upgrade(&admin, &BytesN::from_array(&env, &[0u8; 32]), &salt);
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
