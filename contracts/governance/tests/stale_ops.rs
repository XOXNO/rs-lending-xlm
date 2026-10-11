//! Operations bound at proposal to the governance state they were proposed
//! under: a later ownership handover or emergency action invalidates them,
//! while operations proposed afterwards execute normally.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, Bytes, BytesN, Env, Error, IntoVal, Symbol, Vec};
use stellar_access::ownable;
use stellar_access::role_transfer::{self, PendingTransfer};
use stellar_contract_utils::pausable;
use stellar_governance::timelock::OperationState;

use common::errors::GenericError;
use common::types::PositionLimits;

use crate::access::{CANCELLER_ROLE, EXECUTOR_ROLE, GUARDIAN_ROLE, ORACLE_ROLE, PROPOSER_ROLE};
use crate::constants::{
    TIMELOCK_OPERATION_GRACE_LEDGERS, TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS,
    TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS,
};
use crate::op::{AdminOperation, RoleArgs, TransferOwnershipArgs};
use crate::test_support::{
    read_controller_position_limits, register_with_controller, upload_controller_wasm,
};
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

fn execute_limits(
    gov: &GovernanceClient<'_>,
    controller: &Address,
    limits: &PositionLimits,
    queued: &Queued,
) -> Result<(), Error> {
    let env = &gov.env;
    match gov.try_execute(
        &None,
        controller,
        &Symbol::new(env, "set_position_limits"),
        &vec![env, limits.clone().into_val(env)],
        &BytesN::from_array(env, &[0u8; 32]),
        &queued.salt,
    ) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

fn limits(max_supply_positions: u32) -> PositionLimits {
    PositionLimits {
        max_supply_positions,
        max_borrow_positions: 3,
    }
}

// MC-2 (gov_a G2): was `operation_open_to_any_proposer_survives_a_handover`;
// a routine operation the owner proposed is now bound to the owner epoch.
#[test]
fn routine_operation_the_former_owner_proposed_dies_with_the_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let o2 = Address::generate(&env);
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let routine = queue(&gov, &o1, AdminOperation::SetPositionLimits(limits(4)), 2);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    assert_eq!(
        execute_limits(&gov, &controller, &limits(4), &routine),
        Err(not_authorized())
    );
    assert_eq!(gov.get_operation_state(&routine.id), OperationState::Ready);
}

#[test]
fn independent_proposer_operation_survives_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let o2 = Address::generate(&env);
    let proposer = grant_proposer(&env, &gov, &o1);
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let routine = queue(
        &gov,
        &proposer,
        AdminOperation::SetPositionLimits(limits(4)),
        2,
    );
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    execute_limits(&gov, &controller, &limits(4), &routine)
        .expect("an independent proposer's work survives the handover");
    assert_eq!(gov.get_operation_state(&routine.id), OperationState::Unset);
    assert_eq!(
        read_controller_position_limits(&env, &controller).max_supply_positions,
        4
    );
}

/// Drops every sidecar of `queued`, guard included: the state of an
/// operation scheduled before the governance upgrade that added guards.
fn as_pre_upgrade(env: &Env, gov: &GovernanceClient<'_>, queued: &Queued) {
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(env, &queued.id);
    });
}

// Pre-upgrade operations (gov_a G3): was
// `owner_only_operation_without_an_owner_epoch_record_executes`; an operation
// without a guard record is now bound to owner epoch 0.
#[test]
fn owner_only_operation_without_a_guard_record_dies_with_the_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, guardian) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let legacy = queue(&gov, &o1, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    as_pre_upgrade(&env, &gov, &legacy);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    assert_eq!(execute_self(&gov, &legacy), Err(not_authorized()));
    assert!(!gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

#[test]
fn operation_without_a_guard_record_executes_while_nothing_it_binds_moved() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&o1, &AdminOperation::Unpause);
    let legacy = queue(&gov, &o1, AdminOperation::SetPositionLimits(limits(4)), 1);
    as_pre_upgrade(&env, &gov, &legacy);
    gov.pause(&o1);
    wait_sensitive(&env);

    execute_limits(&gov, &controller, &limits(4), &legacy)
        .expect("a pause does not touch a pre-upgrade routine operation");
    assert_eq!(
        read_controller_position_limits(&env, &controller).max_supply_positions,
        4
    );
}

fn reset_to(env: &Env, account: &Address) -> Vec<Address> {
    vec![env, account.clone()]
}

fn wait_recovery(env: &Env) {
    env.ledger()
        .with_mut(|l| l.sequence_number += TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS);
}

fn execute_reset(
    gov: &GovernanceClient<'_>,
    list: &Vec<Address>,
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

fn cancel_nomination(to: &Address) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: to.clone(),
        live_until_ledger: 0,
    })
}

fn pending_owner(env: &Env, gov: &GovernanceClient<'_>) -> Option<Address> {
    env.as_contract(&gov.address, || {
        env.storage()
            .temporary()
            .get::<_, PendingTransfer>(&ownable::OwnableStorageKey::PendingOwner)
            .map(|pending| pending.address)
    })
}

#[test]
fn nomination_cancel_outrun_by_another_nomination_is_consumed_as_a_no_op() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (x, y) = (Address::generate(&env), Address::generate(&env));
    let nominate_x = queue(&gov, &o1, transfer(&env, &x), 1);
    wait_sensitive(&env);
    execute_self(&gov, &nominate_x).expect("X is nominated");
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    let nominate_y = queue(&gov, &o1, transfer(&env, &y), 3);
    wait_sensitive(&env);
    execute_self(&gov, &nominate_y).expect("Y replaces X");

    execute_self(&gov, &cancel_x).expect("the outrun cancel is consumed");

    assert_eq!(gov.get_operation_state(&cancel_x.id), OperationState::Unset);
    assert_eq!(pending_owner(&env, &gov), Some(y));
}

#[test]
fn nomination_cancel_cannot_void_a_later_nomination_of_the_same_account() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    let first = queue(&gov, &o1, transfer(&env, &x), 1);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("X is nominated");
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    let second = queue(&gov, &o1, transfer(&env, &x), 3);
    wait_sensitive(&env);
    execute_self(&gov, &second).expect("X is nominated again");

    execute_self(&gov, &cancel_x).expect("the stale cancel is consumed");

    assert_eq!(pending_owner(&env, &gov), Some(x.clone()));
    gov.accept_ownership();
    assert_eq!(owner(&env, &gov), x);
}

#[test]
fn nomination_cancel_clears_the_nomination_it_was_proposed_against() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    let nominate_x = queue(&gov, &o1, transfer(&env, &x), 1);
    wait_sensitive(&env);
    execute_self(&gov, &nominate_x).expect("X is nominated");
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    wait_sensitive(&env);

    execute_self(&gov, &cancel_x).expect("the cancel executes");

    assert_eq!(pending_owner(&env, &gov), None);
    assert!(gov.try_accept_ownership().is_err());
}

// Pre-upgrade operations (gov_a G3): was
// `legacy_nomination_cancel_keeps_its_unbound_behaviour`; a cancel without a
// guard record is now bound to nomination epoch 0.
#[test]
fn legacy_nomination_cancel_cannot_void_a_later_nomination() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    let first = queue(&gov, &o1, transfer(&env, &x), 1);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("X is nominated");
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    as_pre_upgrade(&env, &gov, &cancel_x);
    let second = queue(&gov, &o1, transfer(&env, &x), 3);
    wait_sensitive(&env);
    execute_self(&gov, &second).expect("X is nominated again");

    execute_self(&gov, &cancel_x).expect("the stale pre-upgrade cancel is consumed");

    assert_eq!(gov.get_operation_state(&cancel_x.id), OperationState::Unset);
    assert_eq!(pending_owner(&env, &gov), Some(x.clone()));
    gov.accept_ownership();
    assert_eq!(owner(&env, &gov), x);
}

#[test]
fn legacy_nomination_cancel_still_clears_the_nomination_pending_at_the_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    // A nomination made by the code before the upgrade: pending, with no
    // nomination epoch advanced.
    env.as_contract(&gov.address, || {
        role_transfer::transfer_role(
            &env,
            &x,
            &ownable::OwnableStorageKey::PendingOwner,
            env.ledger().sequence() + NOMINATION_WINDOW,
        );
        assert_eq!(storage::nomination_epoch(&env), 0);
    });
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    as_pre_upgrade(&env, &gov, &cancel_x);
    wait_sensitive(&env);

    execute_self(&gov, &cancel_x).expect("the pre-upgrade cancel executes");
    assert_eq!(pending_owner(&env, &gov), None);
    assert!(gov.try_accept_ownership().is_err());
}

fn emergency_epoch_mismatch() -> Error {
    Error::from_contract_error(GenericError::EmergencyEpochMismatch as u32)
}

fn paused(env: &Env, controller: &Address) -> bool {
    env.as_contract(controller, || pausable::paused(env))
}

fn wait_standard(env: &Env) {
    env.ledger().with_mut(|l| l.sequence_number += MIN_DELAY);
}

fn execute_unpause(
    gov: &GovernanceClient<'_>,
    controller: &Address,
    queued: &Queued,
) -> Result<(), Error> {
    let env = &gov.env;
    match gov.try_execute(
        &None,
        controller,
        &Symbol::new(env, "unpause"),
        &Vec::new(env),
        &BytesN::from_array(env, &[0u8; 32]),
        &queued.salt,
    ) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

#[test]
fn unpause_proposed_before_a_later_pause_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let first = queue(&gov, &owner, AdminOperation::Unpause, 1);
    let stale = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_standard(&env);
    execute_unpause(&gov, &controller, &first).expect("the first unpause reopens");
    gov.pause(&owner);

    assert_eq!(
        execute_unpause(&gov, &controller, &stale),
        Err(emergency_epoch_mismatch())
    );
    assert!(paused(&env, &controller));
}

#[test]
fn unpause_proposed_during_the_current_pause_executes() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    gov.pause(&owner);
    let unpause = queue(&gov, &owner, AdminOperation::Unpause, 1);
    wait_standard(&env);

    execute_unpause(&gov, &controller, &unpause).expect("the unpause reopens");
    assert!(!paused(&env, &controller));
}

// Pre-upgrade operations (gov_a G3): was
// `unpause_without_a_guard_record_keeps_its_unbound_behaviour`; an `Unpause`
// without a guard record is now bound to emergency epoch 0.
#[test]
fn unpause_without_a_guard_record_dies_on_a_later_pause() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let first = queue(&gov, &owner, AdminOperation::Unpause, 1);
    let legacy = queue(&gov, &owner, AdminOperation::Unpause, 2);
    as_pre_upgrade(&env, &gov, &legacy);
    wait_standard(&env);
    execute_unpause(&gov, &controller, &first).expect("the first unpause reopens");
    gov.pause(&owner);

    assert_eq!(
        execute_unpause(&gov, &controller, &legacy),
        Err(emergency_epoch_mismatch())
    );
    assert!(paused(&env, &controller));
}

#[test]
fn unpause_without_a_guard_record_reopens_while_no_emergency_followed() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let legacy = queue(&gov, &owner, AdminOperation::Unpause, 1);
    as_pre_upgrade(&env, &gov, &legacy);
    wait_standard(&env);

    execute_unpause(&gov, &controller, &legacy)
        .expect("the reopen pending at the upgrade still applies");
    assert!(!paused(&env, &controller));
}

fn grant_proposer(env: &Env, gov: &GovernanceClient<'_>, owner: &Address) -> Address {
    let rogue = Address::generate(env);
    let op = queue(gov, owner, grant(env, &rogue, PROPOSER_ROLE), 0xF0);
    wait_sensitive(env);
    execute_self(gov, &op).expect("the proposer grant executes");
    rogue
}

/// Executes a Ready `UpgradeController` to the controller's current code.
fn execute_controller_upgrade(gov: &GovernanceClient<'_>, controller: &Address, upgrade: &Queued) {
    let env = &gov.env;
    let AdminOperation::UpgradeController(wasm) = &upgrade.op else {
        panic!("not a controller upgrade");
    };
    gov.execute(
        &None,
        controller,
        &Symbol::new(env, "upgrade"),
        &vec![env, wasm.into_val(env)],
        &BytesN::from_array(env, &[0u8; 32]),
        &upgrade.salt,
    );
}

#[test]
fn unpause_proposed_before_a_controller_upgrade_executes_is_rejected() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let wasm = upload_controller_wasm(&env);
    let upgrade = queue(&gov, &owner, AdminOperation::UpgradeController(wasm), 1);
    let early = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_sensitive(&env);
    execute_controller_upgrade(&gov, &controller, &upgrade);

    assert_eq!(
        execute_unpause(&gov, &controller, &early),
        Err(emergency_epoch_mismatch())
    );
    assert!(paused(&env, &controller));
}

#[test]
fn unpause_proposed_after_a_controller_upgrade_executes_reopens() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let wasm = upload_controller_wasm(&env);
    let upgrade = queue(&gov, &owner, AdminOperation::UpgradeController(wasm), 1);
    wait_sensitive(&env);
    execute_controller_upgrade(&gov, &controller, &upgrade);
    let after = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_standard(&env);

    execute_unpause(&gov, &controller, &after)
        .expect("an unpause proposed after the upgrade reopens");
    assert!(!paused(&env, &controller));
}

#[test]
fn proposing_unpauses_does_not_grow_instance_storage() {
    use soroban_sdk::testutils::storage::Instance as _;
    use soroban_sdk::xdr::ToXdr as _;
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let rogue = grant_proposer(&env, &gov, &owner);
    let instance_bytes = || {
        env.as_contract(&gov.address, || {
            env.storage().instance().all().to_xdr(&env).len()
        })
    };
    queue(&gov, &rogue, AdminOperation::Unpause, 1);
    let before = instance_bytes();

    for byte in 2..=20 {
        queue(&gov, &rogue, AdminOperation::Unpause, byte);
    }

    assert_eq!(instance_bytes(), before);
}

fn role_revoked_after_proposal() -> Error {
    Error::from_contract_error(GenericError::RoleRevokedAfterProposal as u32)
}

#[test]
fn role_grant_queued_before_a_hot_revocation_cannot_rearm_the_key() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let guardian = Address::generate(&env);
    let first = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 1);
    let stale = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("the guardian is granted");

    gov.revoke_role_immediate(&guardian, &Symbol::new(&env, GUARDIAN_ROLE));

    // MC-4 (gov_a G4): the grant is bound to the (account, role) revocation
    // epoch, no longer to the global emergency epoch, hence the new code.
    assert_eq!(
        execute_self(&gov, &stale),
        Err(role_revoked_after_proposal())
    );
    assert!(!gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

#[test]
fn role_grant_proposed_after_a_hot_revocation_executes() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let guardian = Address::generate(&env);
    let first = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 1);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("the guardian is granted");
    gov.revoke_role_immediate(&guardian, &Symbol::new(&env, GUARDIAN_ROLE));

    let regrant = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    wait_sensitive(&env);

    execute_self(&gov, &regrant).expect("a grant proposed after the revocation executes");
    assert!(gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

// Pre-upgrade operations (gov_a G3): was
// `role_grant_without_a_guard_record_keeps_its_unbound_behaviour`; a grant
// without a guard record is now bound to revocation epoch 0 of its pair.
#[test]
fn role_grant_without_a_guard_record_dies_on_a_later_revocation() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let guardian = Address::generate(&env);
    let first = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 1);
    let legacy = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    as_pre_upgrade(&env, &gov, &legacy);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("the guardian is granted");
    gov.revoke_role_immediate(&guardian, &Symbol::new(&env, GUARDIAN_ROLE));

    assert_eq!(
        execute_self(&gov, &legacy),
        Err(role_revoked_after_proposal())
    );
    assert!(!gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}

// ---------------------------------------------------------------------------
// Regression tests ported from the PR #196 verification (governance checks
// g5b, g8, g10a-c, extra-1, extra-2). Each asserts the safe behaviour the
// governance guards now enforce.
// ---------------------------------------------------------------------------

fn revoke(env: &Env, from: &Address, role: &str) -> AdminOperation {
    AdminOperation::RevokeGovRole(RoleArgs {
        account: from.clone(),
        role: Symbol::new(env, role),
    })
}

fn has(gov: &GovernanceClient<'_>, who: &Address, role: &str) -> bool {
    gov.has_role(who, &Symbol::new(&gov.env, role))
}

fn try_cancel(
    gov: &GovernanceClient<'_>,
    canceller: &Address,
    id: &BytesN<32>,
) -> Result<(), Error> {
    match gov.try_cancel(canceller, id) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

fn not_cancellable() -> Error {
    Error::from_contract_error(GenericError::OperationNotCancellable as u32)
}

/// g5b: an older `GrantGovRole` executed after a newer timelocked
/// `RevokeGovRole` of the same key cannot re-arm it; the revocation advances
/// the (account, role) revocation epoch the grant is bound to.
#[test]
fn older_grant_cannot_run_after_a_newer_timelocked_revoke() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let g = Address::generate(&env);
    gov.execute_immediate(&owner, &grant(&env, &g, GUARDIAN_ROLE));
    let older = queue(&gov, &owner, grant(&env, &g, GUARDIAN_ROLE), 5);
    let newer = queue(&gov, &owner, revoke(&env, &g, GUARDIAN_ROLE), 6);
    wait_sensitive(&env);
    execute_self(&gov, &newer).expect("the newer revoke executes");
    assert!(!has(&gov, &g, GUARDIAN_ROLE));

    assert_eq!(
        execute_self(&gov, &older),
        Err(role_revoked_after_proposal())
    );
    assert!(!has(&gov, &g, GUARDIAN_ROLE), "the key stays disarmed");
    assert_eq!(gov.get_operation_state(&older.id), OperationState::Ready);
}

/// The revocation epoch is per (account, role): revoking another account's
/// role, or another role of the same account, leaves a pending grant live.
#[test]
fn revocation_of_another_pair_keeps_a_pending_grant() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (g, other) = (Address::generate(&env), Address::generate(&env));
    gov.execute_immediate(&owner, &grant(&env, &g, ORACLE_ROLE));
    gov.execute_immediate(&owner, &grant(&env, &other, GUARDIAN_ROLE));
    let pending = queue(&gov, &owner, grant(&env, &g, GUARDIAN_ROLE), 1);
    gov.revoke_role_immediate(&g, &Symbol::new(&env, ORACLE_ROLE));
    gov.revoke_role_immediate(&other, &Symbol::new(&env, GUARDIAN_ROLE));
    wait_sensitive(&env);

    execute_self(&gov, &pending).expect("unrelated revocations keep the grant");
    assert!(has(&gov, &g, GUARDIAN_ROLE));
}

/// g8: a `TransferGovOwnership` queued before the governance upgrade (no
/// guard record) cannot hijack ownership after a handover.
#[test]
fn pre_upgrade_transfer_leftover_cannot_hijack_after_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, m1) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let leftover = queue(&gov, &o1, transfer(&env, &m1), 2);
    as_pre_upgrade(&env, &gov, &leftover);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    assert_eq!(execute_self(&gov, &leftover), Err(not_authorized()));
    assert!(
        gov.try_accept_ownership().is_err(),
        "m1 has nothing to accept"
    );
    assert_eq!(owner(&env, &gov), o2);
}

/// g10a: a guardian pause does not void an unrelated pending grant.
#[test]
fn guardian_pause_keeps_an_unrelated_pending_grant() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    let c = Address::generate(&env);
    let pending = queue(&gov, &owner, grant(&env, &c, CANCELLER_ROLE), 1);
    gov.pause(&owner);
    wait_sensitive(&env);

    execute_self(&gov, &pending).expect("a pause keeps an unrelated grant");
    assert!(has(&gov, &c, CANCELLER_ROLE));
}

/// g10b: an executed controller upgrade does not void an unrelated grant.
#[test]
fn controller_upgrade_keeps_an_unrelated_pending_grant() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let c = Address::generate(&env);
    let pending = queue(&gov, &owner, grant(&env, &c, CANCELLER_ROLE), 1);
    let wasm = upload_controller_wasm(&env);
    let upgrade = queue(&gov, &owner, AdminOperation::UpgradeController(wasm), 2);
    wait_sensitive(&env);
    execute_controller_upgrade(&gov, &controller, &upgrade);

    execute_self(&gov, &pending).expect("an upgrade keeps an unrelated grant");
    assert!(has(&gov, &c, CANCELLER_ROLE));
}

/// g10c: an immediate revocation of an unrelated ORACLE key does not void an
/// honest reopen proposed during the pause.
#[test]
fn unrelated_oracle_revocation_keeps_an_honest_reopen() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    let o = Address::generate(&env);
    gov.execute_immediate(&owner, &grant(&env, &o, ORACLE_ROLE));
    gov.pause(&owner);
    let reopen = queue(&gov, &owner, AdminOperation::Unpause, 1);
    gov.revoke_role_immediate(&o, &Symbol::new(&env, ORACLE_ROLE));
    wait_standard(&env);

    execute_unpause(&gov, &controller, &reopen).expect("the honest reopen executes");
    assert!(!paused(&env, &controller));
}

/// extra-1: an A -> A self-transfer keeps the owner epoch, so the owner's own
/// pending work survives it.
#[test]
fn self_transfer_keeps_the_owner_queue() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let guardian = Address::generate(&env);
    let own = queue(&gov, &o1, grant(&env, &guardian, GUARDIAN_ROLE), 1);
    let self_transfer = queue(&gov, &o1, transfer(&env, &o1), 2);
    wait_sensitive(&env);
    hand_over(&gov, &self_transfer, &o1);
    env.as_contract(&gov.address, || {
        assert_eq!(storage::owner_epoch(&env), 0);
    });

    execute_self(&gov, &own).expect("a self-transfer keeps the owner's grant");
    assert!(has(&gov, &guardian, GUARDIAN_ROLE));
}

/// extra-2: a canceller reset voided by a handover can be cancelled at once
/// by the new owner, which frees its id; a live reset still cannot be.
#[test]
fn dead_reset_is_cancellable_after_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, planted) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let list = reset_to(&env, &planted);
    let salt = BytesN::from_array(&env, &[2u8; 32]);
    let id = gov.propose_canceller_reset(&list, &salt);
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    gov.cancel(&o2, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);

    let again = gov.propose_canceller_reset(&list, &salt);
    assert_eq!(again, id, "the freed salt schedules the same id");
    assert_eq!(try_cancel(&gov, &o2, &again), Err(not_cancellable()));
}

/// A reset scheduled before the upgrade (bare `RecoveryOp` marker) is
/// uncancellable while its owner holds office and cancellable once a
/// handover voids it.
#[test]
fn legacy_reset_becomes_cancellable_only_after_a_handover() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, planted) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let salt = BytesN::from_array(&env, &[3u8; 32]);
    let id = gov.propose_canceller_reset(&reset_to(&env, &planted), &salt);
    as_legacy_reset(&env, &gov, &id);
    assert_eq!(try_cancel(&gov, &o1, &id), Err(not_cancellable()));
    wait_sensitive(&env);
    hand_over(&gov, &handover, &o2);

    gov.cancel(&o2, &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

/// The guard check runs only for an operation the timelock holds: replaying
/// an executed operation after a handover still reports the timelock's own
/// state error, not a guard error.
#[test]
fn executed_operation_replayed_after_a_handover_reports_its_timelock_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (o2, guardian) = (Address::generate(&env), Address::generate(&env));
    let handover = queue(&gov, &o1, transfer(&env, &o2), 1);
    let done = queue(&gov, &o1, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    wait_sensitive(&env);
    execute_self(&gov, &done).expect("the grant executes");
    hand_over(&gov, &handover, &o2);

    assert_eq!(
        execute_self(&gov, &done),
        Err(Error::from_contract_error(4002))
    );
}

// ---------------------------------------------------------------------------
// Ordering without a predecessor (gov_b G7: PR #196 verification g5a, g5c).
// An `Unpause` refuses to execute while an `UpgradeController` is pending,
// and every upgrade is bound to the upgrade epoch of the contract it
// replaces, so a build proposed before another upgrade of the same contract
// executed cannot land after it.
// ---------------------------------------------------------------------------

fn controller_upgrade_pending() -> Error {
    Error::from_contract_error(GenericError::ControllerUpgradePending as u32)
}

fn upgrade_superseded() -> Error {
    Error::from_contract_error(GenericError::UpgradeSupersededAfterProposal as u32)
}

fn try_execute_controller_upgrade(
    gov: &GovernanceClient<'_>,
    controller: &Address,
    upgrade: &Queued,
) -> Result<(), Error> {
    let env = &gov.env;
    let AdminOperation::UpgradeController(wasm) = &upgrade.op else {
        panic!("not a controller upgrade");
    };
    match gov.try_execute(
        &None,
        controller,
        &Symbol::new(env, "upgrade"),
        &vec![env, wasm.into_val(env)],
        &BytesN::from_array(env, &[0u8; 32]),
        &upgrade.salt,
    ) {
        Ok(_) => Ok(()),
        Err(Ok(error)) => Err(error),
        Err(Err(invoke)) => panic!("expected a contract error, got {invoke:?}"),
    }
}

/// Uploads the controller WASM with a trailing custom section, so the code
/// is the same but the hash differs.
fn upload_controller_wasm_variant(env: &Env, tag: u8) -> BytesN<32> {
    let path = "target/wasm32v1-none/release/controller.wasm";
    let mut bytes = std::fs::read(path)
        .or_else(|_| std::fs::read(std::format!("../{path}")))
        .or_else(|_| std::fs::read(std::format!("../../{path}")))
        .expect("controller WASM");
    // Custom section: id 0, size 4, name "v2" (length 2), payload [tag].
    bytes.extend_from_slice(&[0x00, 0x04, 0x02, b'v', b'2', tag]);
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}

fn unlimited_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    env
}

/// g5a: the fix (`UpgradeController`) and the reopen (`Unpause`) are both
/// queued during a pause and both Ready. The reopen is refused while the fix
/// is pending and the controller stays paused; once the fix executes, the
/// early reopen is void (it predates that emergency action) and a reopen
/// proposed after the fix executes.
#[test]
fn reopen_cannot_run_before_a_ready_fix() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    gov.pause(&owner);
    let fix = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm(&env)),
        1,
    );
    let reopen = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_sensitive(&env);
    assert_eq!(gov.get_operation_state(&fix.id), OperationState::Ready);
    assert_eq!(gov.get_operation_state(&reopen.id), OperationState::Ready);

    assert_eq!(
        execute_unpause(&gov, &controller, &reopen),
        Err(controller_upgrade_pending())
    );
    assert!(paused(&env, &controller), "the old code stays paused");
    assert_eq!(gov.get_operation_state(&reopen.id), OperationState::Ready);

    try_execute_controller_upgrade(&gov, &controller, &fix).expect("the fix lands");
    assert_eq!(
        execute_unpause(&gov, &controller, &reopen),
        Err(emergency_epoch_mismatch())
    );
    let after = queue(&gov, &owner, AdminOperation::Unpause, 3);
    wait_standard(&env);
    execute_unpause(&gov, &controller, &after).expect("the reopen after the fix executes");
    assert!(!paused(&env, &controller));
}

/// g5c: two `UpgradeController` operations with distinct builds; the newer
/// one executes first and the superseded build is refused.
#[test]
fn superseded_build_cannot_land_after_the_newer_one() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let old_build = upload_controller_wasm(&env);
    let new_build = upload_controller_wasm_variant(&env, 2);
    assert_ne!(old_build, new_build);
    let older = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(old_build),
        1,
    );
    let newer = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(new_build),
        2,
    );
    wait_sensitive(&env);
    try_execute_controller_upgrade(&gov, &controller, &newer).expect("the newer build lands");

    assert_eq!(
        try_execute_controller_upgrade(&gov, &controller, &older),
        Err(upgrade_superseded())
    );
    assert_eq!(gov.get_operation_state(&older.id), OperationState::Ready);
    gov.cancel(&owner, &older.id);
    assert_eq!(gov.get_operation_state(&older.id), OperationState::Unset);
}

/// A cancelled upgrade leaves the pending count, so the reopen it blocked
/// executes; an upgrade proposed after another one executed lands.
#[test]
fn cancelled_upgrade_no_longer_blocks_a_reopen() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    gov.pause(&owner);
    let fix = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm(&env)),
        1,
    );
    let reopen = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_sensitive(&env);
    assert_eq!(
        execute_unpause(&gov, &controller, &reopen),
        Err(controller_upgrade_pending())
    );

    gov.cancel(&owner, &fix.id);
    execute_unpause(&gov, &controller, &reopen).expect("nothing is pending any more");
    assert!(!paused(&env, &controller));

    let later = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm_variant(&env, 3)),
        3,
    );
    wait_sensitive(&env);
    try_execute_controller_upgrade(&gov, &controller, &later)
        .expect("an upgrade proposed after the epoch moved lands");
}

/// Liveness trade-off: a pending upgrade blocks every reopen until it leaves
/// the queue, a voided one included. Here the newer of two builds executes,
/// so the older can never land (62), yet it still counts as pending and the
/// reopen proposed after the fix is refused (61) until a canceller cancels
/// it.
#[test]
fn voided_upgrade_blocks_a_reopen_until_a_canceller_clears_it() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let older = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm(&env)),
        1,
    );
    let newer = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm_variant(&env, 2)),
        2,
    );
    wait_sensitive(&env);
    try_execute_controller_upgrade(&gov, &controller, &newer).expect("the newer build lands");
    let reopen = queue(&gov, &owner, AdminOperation::Unpause, 3);
    wait_standard(&env);

    assert_eq!(
        execute_unpause(&gov, &controller, &reopen),
        Err(controller_upgrade_pending())
    );
    gov.cancel(&owner, &older.id);
    execute_unpause(&gov, &controller, &reopen).expect("the queue holds no upgrade");
    assert!(!paused(&env, &controller));
}

/// Clearing an expired upgrade to re-propose its id keeps the count exact:
/// the cleared operation leaves it and the new one enters it, so one cancel
/// frees the reopen.
#[test]
fn reproposing_an_expired_upgrade_counts_it_once() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    gov.execute_immediate(&owner, &AdminOperation::Unpause);
    gov.pause(&owner);
    let wasm = upload_controller_wasm(&env);
    let first = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(wasm.clone()),
        1,
    );
    env.ledger().with_mut(|l| {
        l.sequence_number += TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS
            .max(MIN_DELAY)
            .saturating_add(TIMELOCK_OPERATION_GRACE_LEDGERS + 1)
    });
    let again = queue(&gov, &owner, AdminOperation::UpgradeController(wasm), 1);
    assert_eq!(again.id, first.id);
    let reopen = queue(&gov, &owner, AdminOperation::Unpause, 2);
    wait_sensitive(&env);
    assert_eq!(
        execute_unpause(&gov, &controller, &reopen),
        Err(controller_upgrade_pending())
    );

    gov.cancel(&owner, &again.id);
    execute_unpause(&gov, &controller, &reopen).expect("one cancel empties the queue");
}

/// An upgrade scheduled before guards existed (no record, not counted as
/// pending) is held to upgrade epoch 0: once a newer build of the same
/// contract executes, it is refused.
#[test]
fn pre_upgrade_controller_upgrade_is_superseded_by_a_newer_one() {
    let env = unlimited_env();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let legacy = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm(&env)),
        1,
    );
    as_pre_upgrade(&env, &gov, &legacy);
    let newer = queue(
        &gov,
        &owner,
        AdminOperation::UpgradeController(upload_controller_wasm_variant(&env, 2)),
        2,
    );
    wait_sensitive(&env);
    try_execute_controller_upgrade(&gov, &controller, &newer).expect("the newer build lands");

    assert_eq!(
        try_execute_controller_upgrade(&gov, &controller, &legacy),
        Err(upgrade_superseded())
    );
}

// ---------------------------------------------------------------------------
// Proposer containment (gov_b G11): the owner revokes PROPOSER and EXECUTOR
// immediately, and an address that loses PROPOSER, by either revocation path
// or a handover away from it as owner, loses its whole queue.
// ---------------------------------------------------------------------------

/// The owner's immediate revocation of a stolen PROPOSER key voids every
/// operation it queued (`NotAuthorized`) while the owner's own queue
/// executes; EXECUTOR is revocable the same way; CANCELLER and the owner's
/// own seats are not.
#[test]
fn immediate_proposer_revocation_voids_its_queue() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let rogue = grant_proposer(&env, &gov, &owner);
    let stolen = queue(
        &gov,
        &rogue,
        AdminOperation::SetPositionLimits(limits(2)),
        1,
    );
    let own = queue(
        &gov,
        &owner,
        AdminOperation::SetPositionLimits(limits(4)),
        2,
    );

    gov.revoke_role_immediate(&rogue, &Symbol::new(&env, PROPOSER_ROLE));
    assert!(!has(&gov, &rogue, PROPOSER_ROLE));
    wait_sensitive(&env);
    assert_eq!(
        execute_limits(&gov, &controller, &limits(2), &stolen),
        Err(not_authorized())
    );
    gov.cancel(&owner, &stolen.id);
    execute_limits(&gov, &controller, &limits(4), &own).expect("the owner's queue is untouched");

    let executor = Address::generate(&env);
    gov.execute_immediate(&owner, &grant(&env, &executor, EXECUTOR_ROLE));
    gov.revoke_role_immediate(&executor, &Symbol::new(&env, EXECUTOR_ROLE));
    assert!(!has(&gov, &executor, EXECUTOR_ROLE));
    for role in [PROPOSER_ROLE, EXECUTOR_ROLE] {
        assert_eq!(
            gov.try_revoke_role_immediate(&owner, &Symbol::new(&env, role)),
            Err(Ok(not_authorized()))
        );
    }
    let canceller = Address::generate(&env);
    gov.execute_immediate(&owner, &grant(&env, &canceller, CANCELLER_ROLE));
    assert_eq!(
        gov.try_revoke_role_immediate(&canceller, &Symbol::new(&env, CANCELLER_ROLE)),
        Err(Ok(Error::from_contract_error(
            GenericError::InvalidRole as u32
        )))
    );
}

/// A timelocked `RevokeGovRole(PROPOSER)` voids the revoked key's queue too,
/// and a re-grant does not revive it: only proposals made after the re-grant
/// execute.
#[test]
fn timelocked_proposer_revocation_voids_its_queue_for_good() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let rogue = grant_proposer(&env, &gov, &owner);
    let stale = queue(
        &gov,
        &rogue,
        AdminOperation::SetPositionLimits(limits(2)),
        1,
    );
    let revocation = queue(&gov, &owner, revoke(&env, &rogue, PROPOSER_ROLE), 2);
    wait_sensitive(&env);
    execute_self(&gov, &revocation).expect("the revocation executes");
    let regrant = queue(&gov, &owner, grant(&env, &rogue, PROPOSER_ROLE), 3);
    wait_sensitive(&env);
    execute_self(&gov, &regrant).expect("the re-grant executes");

    assert_eq!(
        execute_limits(&gov, &controller, &limits(2), &stale),
        Err(not_authorized())
    );
    let fresh = queue(
        &gov,
        &rogue,
        AdminOperation::SetPositionLimits(limits(3)),
        4,
    );
    wait_sensitive(&env);
    execute_limits(&gov, &controller, &limits(3), &fresh)
        .expect("a proposal made after the re-grant executes");
}

/// An address's queue dies when it loses PROPOSER at a handover away from it
/// as owner, including what it proposed before it became owner.
#[test]
fn queue_from_before_the_tenure_dies_with_the_handover_away() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let a = grant_proposer(&env, &gov, &o1);
    let early = queue(&gov, &a, AdminOperation::SetPositionLimits(limits(2)), 1);
    let to_a = queue(&gov, &o1, transfer(&env, &a), 2);
    wait_sensitive(&env);
    hand_over(&gov, &to_a, &a);
    let b = Address::generate(&env);
    let to_b = queue(&gov, &a, transfer(&env, &b), 3);
    wait_sensitive(&env);
    hand_over(&gov, &to_b, &b);

    assert_eq!(
        execute_limits(&gov, &controller, &limits(2), &early),
        Err(not_authorized())
    );
}

// ---------------------------------------------------------------------------
// Review finding GOV-1: a canceller reset proposed before a `CANCELLER`
// revocation of a listed account must not re-seat that account. The reset
// records each listed account's revocation epoch at proposal and leaves out
// an account whose epoch moved since; the rest of the list is seated.
// ---------------------------------------------------------------------------

/// Seats `account` as a canceller through the owner.
fn seat_canceller(env: &Env, gov: &GovernanceClient<'_>, owner: &Address, account: &Address) {
    gov.execute_immediate(owner, &grant(env, account, CANCELLER_ROLE));
}

#[test]
fn canceller_reset_does_not_reseat_an_account_revoked_after_the_proposal() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (a, b, x) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    for account in [&a, &b, &x] {
        seat_canceller(&env, &gov, &owner, account);
    }
    let proposer = grant_proposer(&env, &gov, &owner);
    let list = vec![&env, a.clone(), b.clone(), x.clone()];
    let salt = BytesN::from_array(&env, &[0x61; 32]);
    gov.propose_canceller_reset(&list, &salt);

    let revocation = queue(&gov, &proposer, revoke(&env, &x, CANCELLER_ROLE), 0x62);
    assert_eq!(
        try_cancel(&gov, &x, &revocation.id),
        Err(not_cancellable()),
        "x cannot cancel its own revocation"
    );
    wait_sensitive(&env);
    execute_self(&gov, &revocation).expect("the revocation executes");
    assert!(!has(&gov, &x, CANCELLER_ROLE));
    wait_recovery(&env);

    execute_reset(&gov, &list, &salt).expect("the reset executes");
    assert!(has(&gov, &a, CANCELLER_ROLE));
    assert!(has(&gov, &b, CANCELLER_ROLE));
    assert!(has(&gov, &owner, CANCELLER_ROLE));
    assert!(
        !has(&gov, &x, CANCELLER_ROLE),
        "the revocation after the proposal stands"
    );
}

/// The same reset proposed after the revocation is a fresh decision: it
/// seats the account.
#[test]
fn canceller_reset_proposed_after_a_revocation_seats_the_account() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    seat_canceller(&env, &gov, &owner, &x);
    let revocation = queue(&gov, &owner, revoke(&env, &x, CANCELLER_ROLE), 0x63);
    wait_sensitive(&env);
    execute_self(&gov, &revocation).expect("the revocation executes");
    let list = reset_to(&env, &x);
    let salt = BytesN::from_array(&env, &[0x64; 32]);
    gov.propose_canceller_reset(&list, &salt);
    wait_recovery(&env);

    execute_reset(&gov, &list, &salt).expect("the reset executes");
    assert!(has(&gov, &x, CANCELLER_ROLE));
}

/// A reset scheduled before resets recorded revocation epochs is held to
/// epoch 0 for every listed account, so it does not re-seat an account
/// revoked after the upgrade either.
#[test]
fn legacy_canceller_reset_does_not_reseat_an_account_revoked_since() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let (a, x) = (Address::generate(&env), Address::generate(&env));
    seat_canceller(&env, &gov, &owner, &x);
    let list = vec![&env, a.clone(), x.clone()];
    let salt = BytesN::from_array(&env, &[0x65; 32]);
    let id = gov.propose_canceller_reset(&list, &salt);
    as_legacy_reset(&env, &gov, &id);
    let revocation = queue(&gov, &owner, revoke(&env, &x, CANCELLER_ROLE), 0x66);
    wait_sensitive(&env);
    execute_self(&gov, &revocation).expect("the revocation executes");
    wait_recovery(&env);

    execute_reset(&gov, &list, &salt).expect("a legacy reset executes under its owner");
    assert!(has(&gov, &a, CANCELLER_ROLE));
    assert!(!has(&gov, &x, CANCELLER_ROLE));
}
