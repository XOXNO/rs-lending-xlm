//! Operations bound at proposal to the governance state they were proposed
//! under: a later ownership handover or emergency action invalidates them,
//! while operations proposed afterwards execute normally.

extern crate std;

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Env, Error, IntoVal, Symbol, Vec};
use stellar_access::ownable;
use stellar_access::role_transfer::PendingTransfer;
use stellar_contract_utils::pausable;
use stellar_governance::timelock::OperationState;

use common::errors::GenericError;
use common::types::PositionLimits;

use crate::access::{CANCELLER_ROLE, GUARDIAN_ROLE, PROPOSER_ROLE};
use crate::constants::{TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS, TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS};
use crate::op::{AdminOperation, RoleArgs, TransferOwnershipArgs};
use crate::test_support::{register_with_controller, upload_controller_wasm};
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
        &vec![&env, limits.into_val(&env)],
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

#[test]
fn legacy_nomination_cancel_keeps_its_unbound_behaviour() {
    let env = Env::default();
    env.mock_all_auths();
    let (o1, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let x = Address::generate(&env);
    let first = queue(&gov, &o1, transfer(&env, &x), 1);
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("X is nominated");
    let cancel_x = queue(&gov, &o1, cancel_nomination(&x), 2);
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(&env, &cancel_x.id);
    });
    let second = queue(&gov, &o1, transfer(&env, &x), 3);
    wait_sensitive(&env);
    execute_self(&gov, &second).expect("X is nominated again");

    execute_self(&gov, &cancel_x).expect("a cancel scheduled before the upgrade still applies");

    assert_eq!(pending_owner(&env, &gov), None);
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

#[test]
fn unpause_without_a_guard_record_keeps_its_unbound_behaviour() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, controller, gov) = register_with_controller(&env, MIN_DELAY);
    let first = queue(&gov, &owner, AdminOperation::Unpause, 1);
    let legacy = queue(&gov, &owner, AdminOperation::Unpause, 2);
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(&env, &legacy.id);
    });
    wait_standard(&env);
    execute_unpause(&gov, &controller, &first).expect("the first unpause reopens");
    gov.pause(&owner);

    execute_unpause(&gov, &controller, &legacy)
        .expect("an unpause scheduled before the upgrade still applies");
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

    assert_eq!(execute_self(&gov, &stale), Err(emergency_epoch_mismatch()));
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

#[test]
fn role_grant_without_a_guard_record_keeps_its_unbound_behaviour() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, _controller, gov) = register_with_controller(&env, MIN_DELAY);
    let guardian = Address::generate(&env);
    let first = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 1);
    let legacy = queue(&gov, &owner, grant(&env, &guardian, GUARDIAN_ROLE), 2);
    env.as_contract(&gov.address, || {
        storage::clear_operation_sidecars(&env, &legacy.id);
    });
    wait_sensitive(&env);
    execute_self(&gov, &first).expect("the guardian is granted");
    gov.revoke_role_immediate(&guardian, &Symbol::new(&env, GUARDIAN_ROLE));

    execute_self(&gov, &legacy).expect("a grant scheduled before the upgrade still applies");
    assert!(gov.has_role(&guardian, &Symbol::new(&env, GUARDIAN_ROLE)));
}
