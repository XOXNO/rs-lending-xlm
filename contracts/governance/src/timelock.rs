//! Timelock execution and query surface backed by `stellar-governance` storage.
//!
//! Operations are queued through typed `propose_*` entrypoints in `forward.rs`;
//! generic `Timelock::schedule` is not exposed. This module provides `execute`,
//! `cancel`, and query views over the OZ operation state machine.
//!
//! `execute` requires EXECUTOR auth when `executor` is `Some`; `None` keeps
//! execution open. `cancel` requires CANCELLER. Controller-targeted operations
//! call `execute_operation`; governance-self operations use typed inline
//! dispatch in `self_timelock.rs` because Soroban prohibits self-reentry.

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE};
use crate::storage::renew_governance_instance;
use crate::{Governance, GovernanceArgs, GovernanceClient};
use common::errors::GenericError;
use controller_interface::types::{
    MarketOracleConfig, MarketOracleConfigInput, OraclePriceFluctuation,
};
use soroban_sdk::{assert_with_error, contractimpl, Address, BytesN, Env, Symbol, Val, Vec};
use stellar_access::access_control;
use stellar_governance::timelock::{
    cancel_operation, execute_operation, get_min_delay, get_operation_ledger, get_operation_state,
    hash_operation, Operation, OperationState,
};

/// Rejects a zero minimum timelock delay, which would nullify the timelock.
pub(crate) fn require_nonzero_delay(env: &Env, delay: u32) {
    assert_with_error!(env, delay >= 1, GenericError::InvalidTimelockDelay);
}

/// Delay updates must not shorten the timelock window.
pub(crate) fn validate_delay_update(env: &Env, new_delay: u32) {
    require_nonzero_delay(env, new_delay);
    let current = get_min_delay(env);
    assert_with_error!(
        env,
        new_delay >= current,
        GenericError::InvalidTimelockDelay
    );
}

pub(crate) fn apply_update_delay(env: &Env, new_delay: u32) {
    validate_delay_update(env, new_delay);
    stellar_governance::timelock::set_min_delay(env, new_delay);
}

pub(crate) fn authorize_executor(env: &Env, executor: Option<&Address>) {
    if let Some(exec) = executor {
        exec.require_auth();
        access_control::ensure_role(env, &Symbol::new(env, EXECUTOR_ROLE), exec);
    }
}

pub(crate) fn require_operation_not_expired(env: &Env, operation: &Operation) {
    let operation_id = hash_operation(env, operation);
    let ready_ledger = get_operation_ledger(env, &operation_id);
    if ready_ledger <= 1 {
        return;
    }

    let expires_at =
        ready_ledger.saturating_add(crate::constants::TIMELOCK_OPERATION_GRACE_LEDGERS);
    assert_with_error!(
        env,
        env.ledger().sequence() <= expires_at,
        GenericError::TimelockOperationExpired
    );
}

#[contractimpl]
impl Governance {
    /// Executes a ready controller operation. When `executor` is `Some`, that
    /// address must hold EXECUTOR and authorize; `None` allows open execution.
    pub fn execute(
        env: Env,
        executor: Option<Address>,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> Val {
        renew_governance_instance(&env);
        authorize_executor(&env, executor.as_ref());
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
        require_operation_not_expired(&env, &operation);
        execute_operation(&env, &operation)
    }

    /// Cancels a pending operation. The caller must hold CANCELLER.
    pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
        renew_governance_instance(&env);
        canceller.require_auth();
        access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
        cancel_operation(&env, &operation_id);
    }

    /// Minimum timelock delay in ledgers.
    pub fn get_min_delay(env: Env) -> u32 {
        get_min_delay(&env)
    }

    /// Current lifecycle state of an operation.
    pub fn get_operation_state(env: Env, operation_id: BytesN<32>) -> OperationState {
        get_operation_state(&env, &operation_id)
    }

    /// Ledger at which an operation becomes ready (`0` unset, `1` done).
    pub fn get_operation_ledger(env: Env, operation_id: BytesN<32>) -> u32 {
        get_operation_ledger(&env, &operation_id)
    }

    /// Deterministic operation id for the given fields.
    pub fn hash_operation(
        env: Env,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        predecessor: BytesN<32>,
        salt: BytesN<32>,
    ) -> BytesN<32> {
        let operation = Operation {
            target,
            function,
            args,
            predecessor,
            salt,
        };
        hash_operation(&env, &operation)
    }

    /// Resolves a market oracle input to the `MarketOracleConfig` scheduled by
    /// `propose_configure_market_oracle`. Uses the proposer's validation and
    /// live oracle probes so simulation can replay the returned args at execute.
    pub fn resolve_market_oracle_config(
        env: Env,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) -> MarketOracleConfig {
        crate::forward::resolve_market_oracle(&env, &asset, &cfg)
    }

    /// Resolves tolerance BPS inputs to the `OraclePriceFluctuation` scheduled
    /// by `propose_edit_oracle_tolerance`. Uses the proposer's computation.
    pub fn resolve_oracle_tolerance(
        env: Env,
        first_tolerance: u32,
        last_tolerance: u32,
    ) -> OraclePriceFluctuation {
        crate::validate::tolerance::validate_and_calculate_tolerances(
            &env,
            first_tolerance,
            last_tolerance,
        )
    }
}

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol};
    use stellar_governance::timelock::OperationState;

    use controller_interface::types::{ControllerKey, PositionLimits};

    use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
    use crate::constants::{TIMELOCK_MIN_DELAY_LEDGERS, TIMELOCK_OPERATION_GRACE_LEDGERS};
    use crate::{Governance, GovernanceClient};

    const ZERO_SALT: [u8; 32] = [0u8; 32];

    fn register(env: &Env, min_delay: u32) -> (Address, GovernanceClient<'_>) {
        let admin = Address::generate(env);
        let gov_id = env.register(Governance, (admin.clone(), min_delay));
        (admin, GovernanceClient::new(env, &gov_id))
    }

    fn register_with_controller(
        env: &Env,
        min_delay: u32,
    ) -> (Address, Address, GovernanceClient<'_>) {
        let (admin, gov) = register(env, min_delay);
        let controller_id = env.register(controller::Controller, (gov.address.clone(),));
        gov.set_controller(&controller_id);
        (admin, controller_id, gov)
    }

    fn read_position_limits(env: &Env, controller_id: &Address) -> PositionLimits {
        env.as_contract(controller_id, || {
            env.storage()
                .instance()
                .get(&ControllerKey::PositionLimits)
                .expect("position limits set")
        })
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

    #[test]
    fn propose_schedules_waiting_operation() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, _controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);

        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #4002)")]
    fn execute_before_delay_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let _ = gov.propose_set_position_limits(&admin, &limits, &salt);

        gov.execute(
            &Some(admin.clone()),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );
    }

    #[test]
    fn execute_after_delay_applies_to_controller() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 6,
            max_borrow_positions: 3,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);
        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

        env.ledger().with_mut(|l| l.sequence_number += delay);
        assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

        gov.execute(
            &Some(admin.clone()),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );

        assert_eq!(gov.get_operation_state(&id), OperationState::Done);
        let stored = read_position_limits(&env, &controller);
        assert_eq!(stored.max_supply_positions, 6);
        assert_eq!(stored.max_borrow_positions, 3);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #40)")]
    fn execute_after_grace_period_reverts() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);

        let limits = PositionLimits {
            max_supply_positions: 6,
            max_borrow_positions: 3,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let _id = gov.propose_set_position_limits(&admin, &limits, &salt);

        env.ledger()
            .with_mut(|l| l.sequence_number += delay + TIMELOCK_OPERATION_GRACE_LEDGERS + 1);

        gov.execute(
            &Some(admin.clone()),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn propose_rejects_bad_input_at_schedule_time() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);

        let limits = PositionLimits {
            max_supply_positions: 0,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&admin, &limits, &salt);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #41)")]
    fn propose_controller_role_rejects_unknown_role() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);

        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_grant_controller_role(&admin, &admin, &Symbol::new(&env, "BAD_ROLE"), &salt);
    }

    #[test]
    fn cancel_returns_operation_to_unset() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);
        assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

        gov.cancel(&admin, &id);
        assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
    }

    #[test]
    #[should_panic]
    fn non_proposer_cannot_propose() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, _controller, gov) = register_with_controller(&env, 10);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&stranger, &limits, &salt);
    }

    #[test]
    #[should_panic]
    fn non_executor_cannot_execute() {
        let env = Env::default();
        env.mock_all_auths();
        let delay = 10u32;
        let (admin, controller, gov) = register_with_controller(&env, delay);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        gov.propose_set_position_limits(&admin, &limits, &salt);
        env.ledger().with_mut(|l| l.sequence_number += delay);

        gov.execute(
            &Some(stranger),
            &controller,
            &Symbol::new(&env, "set_position_limits"),
            &vec![&env, limits.into_val(&env)],
            &BytesN::<32>::from_array(&env, &ZERO_SALT),
            &salt,
        );
    }

    #[test]
    #[should_panic]
    fn non_canceller_cannot_cancel() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _controller, gov) = register_with_controller(&env, 10);
        let stranger = Address::generate(&env);

        let limits = PositionLimits {
            max_supply_positions: 5,
            max_borrow_positions: 4,
        };
        let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
        let id = gov.propose_set_position_limits(&admin, &limits, &salt);

        gov.cancel(&stranger, &id);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #39)")]
    fn constructor_rejects_zero_delay() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let _ = env.register(Governance, (admin, 0u32));
    }
}
