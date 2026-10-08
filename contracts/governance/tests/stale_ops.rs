//! Operations bound at proposal to the governance state they were proposed
//! under: a later ownership handover or emergency action invalidates them,
//! while operations proposed afterwards execute normally.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, Error, Symbol};
use stellar_access::ownable;
use stellar_governance::timelock::OperationState;

use common::errors::GenericError;
use common::types::PositionLimits;

use crate::access::{CANCELLER_ROLE, GUARDIAN_ROLE, PROPOSER_ROLE};
use crate::constants::{TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS, TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS};
use crate::op::{AdminOperation, RoleArgs, TransferOwnershipArgs};
use crate::test_support::register_with_controller;
use crate::{storage, GovernanceClient};

/// Minimum delay configured on testnet and mainnet.
const MIN_DELAY: u32 = 12;
/// Ledgers a nomination stays acceptable.
const NOMINATION_WINDOW: u32 = 10_000;

struct Queued {
    op: AdminOperation,
    salt: BytesN<32>,
    id: BytesN<32>,
}

fn queue(gov: &GovernanceClient<'_>, proposer: &Address, op: AdminOperation, byte: u8) -> Queued {
    let salt = BytesN::from_array(&gov.env, &[byte; 32]);
    let id = gov.propose(proposer, &op, &salt);
    Queued { op, salt, id }
}

fn wait_sensitive(env: &Env) {
    env.ledger()
        .with_mut(|l| l.sequence_number += MIN_DELAY.max(TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS));
}

fn execute_self(gov: &GovernanceClient<'_>, queued: &Queued) -> Result<(), Error> {
    match gov.try_execute_self(&None, &queued.op, &queued.salt) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

fn not_authorized() -> Error {
    Error::from_contract_error(GenericError::NotAuthorized as u32)
}

fn transfer(env: &Env, to: &Address) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: to.clone(),
        live_until_ledger: env.ledger().sequence() + NOMINATION_WINDOW,
    })
}

fn grant(env: &Env, to: &Address, role: &str) -> AdminOperation {
    AdminOperation::GrantGovRole(RoleArgs {
        account: to.clone(),
        role: Symbol::new(env, role),
    })
}

fn owner(env: &Env, gov: &GovernanceClient<'_>) -> Address {
    env.as_contract(&gov.address, || ownable::get_owner(env).expect("owner set"))
}

/// Executes the queued handover and has `to` accept it.
fn hand_over(gov: &GovernanceClient<'_>, handover: &Queued, to: &Address) {
    execute_self(gov, handover).expect("the handover executes");
    gov.accept_ownership();
    assert_eq!(&owner(&gov.env, gov), to);
}

#[test]
fn former_owner_transfer_queued_before_a_handover_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, m1) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let leftover = queue(&gov, &o1, transfer(&env, &m1), 2);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    assert_eq!(execute_self(&gov, &leftover), Err(not_authorized()));
    assert_eq!(gov.get_operation_state(&leftover.id), OperationState::Ready);
    assert_eq!(owner(&env, &gov), o2);
}

#[test]
fn former_owner_grants_queued_before_a_handover_are_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let o2 = Address::generate(&env);
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let canceller = queue(&gov, &o1, grant(&env, &o1, CANCELLER_ROLE), 2);
    let proposer = queue(&gov, &o1, grant(&env, &o1, PROPOSER_ROLE), 3);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    assert_eq!(execute_self(&gov, &canceller), Err(not_authorized()));
    assert_eq!(execute_self(&gov, &proposer), Err(not_authorized()));
    assert!(!gov.has_role(&o1, &Symbol::new(&env, CANCELLER_ROLE)));
    assert!(!gov.has_role(&o1, &Symbol::new(&env, PROPOSER_ROLE)));
}

#[test]
fn owner_only_operation_proposed_after_a_handover_executes() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, guardian) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    let fresh = queue(&gov, &o2, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    wait_sensitive(&env);

    execute_self(&gov, &fresh).expect("the new owner's grant executes");
    assert!(gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

#[test]
fn operation_open_to_any_proposer_survives_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let o2 = Address::generate(&env);
    let limits = PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 3,
    };
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let routine = queue(
        &gov,
        &o1,
        AdminOperation::SetPositionLimits(limits.clone()),
        2,
    );
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    gov.execute(
        &None,
        &controller,
        &Symbol::new(&env, "set_position_limits"),
        &soroban_sdk::vec![&env, soroban_sdk::IntoVal::into_val(&limits, &env)],
        &BytesN::from_array(&env, &[0u8; 32]),
        &routine.salt,
    );
    assert_eq!(gov.get_operation_state(&routine.id), OperationState::Unset);
}

#[test]
fn owner_only_operation_without_an_owner_epoch_record_executes() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, guardian) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let legacy = queue(&gov, &o1, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(&env, &legacy.id);
    });
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    execute_self(&gov, &legacy).expect("an operation scheduled before the upgrade executes");
    assert!(gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

fn reset_to(env: &Env, account: &Address) -> soroban_sdk::Vec<Address> {
    soroban_sdk::vec![env, account.clone()]
}

fn wait_recovery(env: &Env) {
    env.ledger()
        .with_mut(|l| l.sequence_number += TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS);
}

fn execute_reset(
    gov: &GovernanceClient<'_>,
    list: &soroban_sdk::Vec<Address>,
    salt: &BytesN<32>,
) -> Result<(), Error> {
    match gov.try_execute_canceller_reset(&None, list, salt) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

#[test]
fn former_owner_canceller_reset_is_rejected_after_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, planted) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let list = reset_to(&env, &planted);
    let salt = BytesN::from_array(&env, &[2u8; 32]);
    gov.propose_canceller_reset(&list, &salt);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);
    wait_recovery(&env);

    assert_eq!(execute_reset(&gov, &list, &salt), Err(not_authorized()));
    let canceller = Symbol::new(&env, CANCELLER_ROLE);
    assert!(!gov.has_role(&planted, &canceller));
    assert!(gov.has_role(&o2, &canceller));
}

#[test]
fn canceller_reset_proposed_by_the_new_owner_executes() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, chosen) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);
    let list = reset_to(&env, &chosen);
    let salt = BytesN::from_array(&env, &[2u8; 32]);
    gov.propose_canceller_reset(&list, &salt);
    wait_recovery(&env);

    execute_reset(&gov, &list, &salt).expect("the new owner's reset executes");
    assert!(gov.has_role(&chosen, &Symbol::new(&env, CANCELLER_ROLE)));
}

/// Rewrites a reset's sidecars to the form stored before resets recorded an
/// owner epoch: the bare `RecoveryOp` marker.
fn as_legacy_reset(env: &Env, gov: &GovernanceClient<'_>, id: &BytesN<32>) {
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(env, id);
        storage::mark_recovery_op(env, id);
    });
}

#[test]
fn legacy_canceller_reset_executes_while_the_owner_is_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let (_o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let chosen = Address::generate(&env);
    let list = reset_to(&env, &chosen);
    let salt = BytesN::from_array(&env, &[2u8; 32]);
    let id = gov.propose_canceller_reset(&list, &salt);
    as_legacy_reset(&env, &gov, &id);
    wait_recovery(&env);

    execute_reset(&gov, &list, &salt).expect("a legacy reset executes under its owner");
    assert!(gov.has_role(&chosen, &Symbol::new(&env, CANCELLER_ROLE)));
}

#[test]
fn legacy_canceller_reset_is_rejected_after_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, planted) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let list = reset_to(&env, &planted);
    let salt = BytesN::from_array(&env, &[2u8; 32]);
    let id = gov.propose_canceller_reset(&list, &salt);
    as_legacy_reset(&env, &gov, &id);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);
    wait_recovery(&env);

    assert_eq!(execute_reset(&gov, &list, &salt), Err(not_authorized()));
    assert!(!gov.has_role(&planted, &Symbol::new(&env, CANCELLER_ROLE)));
}
