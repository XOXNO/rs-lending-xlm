use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Env, IntoVal, Symbol};
use stellar_governance::timelock::OperationState;

use common::types::{ControllerKey, PositionLimits};

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, GUARDIAN_ROLE, PROPOSER_ROLE};
use crate::constants::{
    TIMELOCK_MAX_DELAY_LEDGERS, TIMELOCK_MIN_DELAY_LEDGERS, TIMELOCK_OPERATION_GRACE_LEDGERS,
    TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS,
};
use crate::op::{AdminOperation, RoleArgs, TransferOwnershipArgs};
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

/// Grants `role` to a fresh (non-owner) address through the timelocked
/// `GrantGovRole` path and returns it. `salt_byte` must be unique per call.
fn grant_role_via_timelock(
    env: &Env,
    gov: &GovernanceClient<'_>,
    admin: &Address,
    delay: u32,
    role: &str,
    salt_byte: u8,
) -> Address {
    let account = Address::generate(env);
    let salt = BytesN::<32>::from_array(env, &[salt_byte; 32]);
    let grant = AdminOperation::GrantGovRole(RoleArgs {
        account: account.clone(),
        role: Symbol::new(env, role),
    });
    gov.propose(admin, &grant, &salt);
    env.ledger().with_mut(|l| l.sequence_number += delay);
    gov.execute_self(&Some(admin.clone()), &grant, &salt);
    account
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

// GUARDIAN pauses the controller immediately; a non-guardian is rejected.
#[test]
fn guardian_pauses_immediately_stranger_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, controller, gov) = register_with_controller(&env, 10);
    let stranger = Address::generate(&env);

    // The controller deploys paused; resume it so a fresh pause is observable.
    gov.execute_immediate(&admin, &AdminOperation::Unpause);

    // A caller without GUARDIAN is rejected by the role gate.
    assert!(gov.try_pause(&stranger).is_err());

    // The admin holds GUARDIAN (constructor) and can halt without the timelock.
    gov.pause(&admin);
    assert!(env.as_contract(&controller, || {
        stellar_contract_utils::pausable::paused(&env)
    }));
}

// Global unpause rides the timelock at the Standard delay (risk-loosening).
#[test]
fn unpause_uses_standard_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);

    let current = env.ledger().sequence();
    let id = gov.propose(&admin, &AdminOperation::Unpause, &salt);
    assert_eq!(gov.get_operation_ledger(&id), current + delay);
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

// A canceller cannot veto the revocation of its own CANCELLER role — no one
// blocks their own removal. Uses a non-owner canceller (the owner's roles are
// never revocable).
#[test]
#[should_panic(expected = "Error(Contract, #46)")]
fn revocation_target_cannot_cancel_own_role_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let canceller = grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 1);

    let salt = BytesN::<32>::from_array(&env, &ZERO_SALT);
    let id = gov.propose(
        &admin,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: canceller.clone(),
            role: Symbol::new(&env, CANCELLER_ROLE),
        }),
        &salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    // The target canceller cannot veto its own removal.
    gov.cancel(&canceller, &id);
}

#[test]
fn independent_canceller_can_cancel_non_canceller_role_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let honest_canceller =
        grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 1);
    // A second (non-owner) PROPOSER so its revocation clears the last-proposer guard.
    let extra_proposer = grant_role_via_timelock(&env, &gov, &admin, delay, PROPOSER_ROLE, 2);

    // A revocation of a NON-canceller role stays vetoable by an independent canceller.
    let revoke_salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    let id = gov.propose(
        &admin,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: extra_proposer,
            role: Symbol::new(&env, PROPOSER_ROLE),
        }),
        &revoke_salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    gov.cancel(&honest_canceller, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

// A CANCELLER-role revocation is now vetoable by an INDEPENDENT canceller (only
// the target itself is barred): the independent-canceller veto stays a real
// check on a rogue proposer trying to strip cancellers. The colluding-canceller
// deadlock this opens is broken by the owner's immediate revoke, tested below.
#[test]
fn independent_canceller_can_veto_canceller_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let target = grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 1);
    let independent = grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 2);

    let revoke_salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    let id = gov.propose(
        &admin,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: target,
            role: Symbol::new(&env, CANCELLER_ROLE),
        }),
        &revoke_salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    // The independent canceller (not the target) can veto it.
    gov.cancel(&independent, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}
// The owner can no longer instantly strip a canceller; CANCELLER revocation
// rides the timelock (single-vetoable) instead. Closes the "owner instantly
// strips canceller vetoes" finding.
#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn owner_cannot_immediately_revoke_canceller() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let canceller = grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 1);
    gov.revoke_role_immediate(&canceller, &Symbol::new(&env, CANCELLER_ROLE));
}

// A proposer cannot revoke its own role, and the owner's roles are never
// revocable — both rejected at propose with NotAuthorized (#44).
#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn proposer_cannot_revoke_itself() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let proposer = grant_role_via_timelock(&env, &gov, &admin, delay, PROPOSER_ROLE, 1);

    let salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    gov.propose(
        &proposer,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: proposer.clone(),
            role: Symbol::new(&env, PROPOSER_ROLE),
        }),
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn owner_role_cannot_be_revoked() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let proposer = grant_role_via_timelock(&env, &gov, &admin, delay, PROPOSER_ROLE, 1);

    let salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    gov.propose(
        &proposer,
        &AdminOperation::RevokeGovRole(RoleArgs {
            account: admin.clone(),
            role: Symbol::new(&env, GUARDIAN_ROLE),
        }),
        &salt,
    );
}

// Only the owner may initiate an ownership transfer; a non-owner proposer is
// rejected at propose (#44). The owner's transfer stays cancellable.
#[test]
#[should_panic(expected = "Error(Contract, #44)")]
fn non_owner_cannot_propose_ownership_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let proposer = grant_role_via_timelock(&env, &gov, &admin, delay, PROPOSER_ROLE, 1);
    let new_owner = Address::generate(&env);

    let salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    gov.propose(
        &proposer,
        &AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
            new_owner,
            live_until_ledger: env.ledger().sequence() + 100_000,
        }),
        &salt,
    );
}

#[test]
fn owner_ownership_transfer_is_cancellable() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let canceller = grant_role_via_timelock(&env, &gov, &admin, delay, CANCELLER_ROLE, 1);
    let new_owner = Address::generate(&env);

    let salt = BytesN::<32>::from_array(&env, &[3u8; 32]);
    let id = gov.propose(
        &admin,
        &AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
            new_owner,
            live_until_ledger: env.ledger().sequence() + 100_000,
        }),
        &salt,
    );
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    gov.cancel(&canceller, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

// Revoking the SOLE PROPOSER reverts (#48): it is the only gate on `propose`, so
// zeroing it would leave no way to schedule any recovery — a permanent freeze.
// (With the owner's roles now unrevocable, the owner always remains a proposer,
// so this guard is a defense-in-depth backstop; the primary protection against
// stripping the owner is covered by `owner_role_cannot_be_revoked`.)

// Revoking a non-owner PROPOSER is allowed while the owner (a proposer) remains.
#[test]
fn revoking_proposer_ok_when_another_remains() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _controller, gov) = register_with_controller(&env, delay);
    let proposer = Symbol::new(&env, PROPOSER_ROLE);
    let second = grant_role_via_timelock(&env, &gov, &admin, delay, PROPOSER_ROLE, 1);

    let r_salt = BytesN::<32>::from_array(&env, &[2u8; 32]);
    let revoke = AdminOperation::RevokeGovRole(RoleArgs {
        account: second.clone(),
        role: proposer.clone(),
    });
    gov.propose(&admin, &revoke, &r_salt);
    env.ledger().with_mut(|l| l.sequence_number += delay);
    gov.execute_self(&Some(admin.clone()), &revoke, &r_salt);
    assert!(!gov.has_role(&second, &proposer));
    assert!(gov.has_role(&admin, &proposer));
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
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
#[should_panic(expected = "Error(Contract, #2000)")]
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
#[should_panic(expected = "Error(Contract, #2000)")]
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
