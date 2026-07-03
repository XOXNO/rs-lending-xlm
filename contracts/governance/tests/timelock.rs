use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol};
use stellar_governance::timelock::OperationState;

use common::types::{ControllerKey, PositionLimits};

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, PROPOSER_ROLE};
use crate::constants::{
    TIMELOCK_MAX_DELAY_LEDGERS, TIMELOCK_MIN_DELAY_LEDGERS, TIMELOCK_OPERATION_GRACE_LEDGERS,
    TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS,
};
use crate::op::{AdminOperation, RoleArgs};
use crate::timelock::{operation_delay, DelayTier};
use crate::{Governance, GovernanceClient};

const ZERO_SALT: [u8; 32] = [0u8; 32];

fn register(env: &Env, min_delay: u32) -> (Address, GovernanceClient<'_>) {
    let admin = Address::generate(env);
    let gov_id = env.register(Governance, (admin.clone(), min_delay));
    (admin, GovernanceClient::new(env, &gov_id))
}

fn register_with_controller(env: &Env, min_delay: u32) -> (Address, Address, GovernanceClient<'_>) {
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
    let id = gov.propose(&admin, &AdminOperation::SetPositionLimits(limits), &salt);

    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
}

#[test]
fn propose_upgrade_pool_uses_sensitive_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _controller, gov) = register_with_controller(&env, 10);
    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
    let hash = BytesN::from_array(&env, &[8u8; 32]);

    let current = env.ledger().sequence();
    let id = gov.propose(&admin, &AdminOperation::UpgradePool(hash), &salt);

    assert_eq!(
        gov.get_operation_ledger(&id),
        current + TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS
    );
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
    let _ = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(limits.clone()),
        &salt,
    );

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
    let id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(limits.clone()),
        &salt,
    );
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
    let _id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(limits.clone()),
        &salt,
    );

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
    gov.propose(&admin, &AdminOperation::SetPositionLimits(limits), &salt);
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
    let id = gov.propose(&admin, &AdminOperation::SetPositionLimits(limits), &salt);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    gov.cancel(&admin, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

#[test]
#[should_panic(expected = "Error(Contract, #46)")]
fn cannot_cancel_role_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _controller, gov) = register_with_controller(&env, 10);

    // A pending revocation of the canceller's own role...
    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
    let id = gov.propose(
        &admin,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: admin.clone(),
            role: Symbol::new(&env, CANCELLER_ROLE),
        }),
        &salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    // ...cannot be vetoed by that canceller, so governance stays recoverable.
    gov.cancel(&admin, &id);
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
    gov.propose(&stranger, &AdminOperation::SetPositionLimits(limits), &salt);
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
    gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(limits.clone()),
        &salt,
    );
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
    let id = gov.propose(&admin, &AdminOperation::SetPositionLimits(limits), &salt);

    gov.cancel(&stranger, &id);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn constructor_rejects_zero_delay() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let _ = env.register(Governance, (admin, 0u32));
}

#[test]
fn operation_delay_sensitive_uses_seven_day_floor() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, gov) = register(&env, 10);

    env.as_contract(&gov.address, || {
        assert_eq!(
            operation_delay(&env, DelayTier::Sensitive),
            TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS
        );
    });
}

#[test]
fn operation_delay_sensitive_respects_higher_global_min() {
    let env = Env::default();
    env.mock_all_auths();
    let higher_min = TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS + 1_000;
    let (_admin, gov) = register(&env, higher_min);

    env.as_contract(&gov.address, || {
        assert_eq!(operation_delay(&env, DelayTier::Sensitive), higher_min);
    });
}

#[test]
fn validate_delay_update_accepts_max_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);
    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);

    let id = gov.propose(
        &admin,
        &AdminOperation::UpdateGovDelay(TIMELOCK_MAX_DELAY_LEDGERS),
        &salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn validate_delay_update_rejects_above_max_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, gov) = register(&env, TIMELOCK_MIN_DELAY_LEDGERS);
    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
    let over_max = TIMELOCK_MAX_DELAY_LEDGERS + 1;

    gov.propose(&admin, &AdminOperation::UpdateGovDelay(over_max), &salt);
}
