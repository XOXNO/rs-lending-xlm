//! Second opinion on Specula governance MC-4 (the `Unpause` arm) and MC-5: a
//! queued `Unpause` is bound to nothing but its own delay, and `propose` hashes
//! every operation with a zero predecessor, so the permissionless executor can
//! run ready operations in any order.
//!
//! `op::resolve_op` maps `Unpause` to a bare controller `unpause` call with no
//! epoch, and `controller::governance::unpause` only requires the controller to
//! be paused (`ExpectedPause`, 1001). A failed execute rolls back the Done
//! write made by `set_execute_operation`, so an `Unpause` proposed while the
//! controller is open stays Ready for the whole grace window and becomes live
//! the moment a guardian pauses. `timelock::operation_for_admin_op` hardcodes
//! `predecessor = 0` and `finish_execute` erases the Done marker, so no
//! operation can be ordered after another on chain.
//!
//! Every step uses production entry points: `propose` as the owner/PROPOSER,
//! the guardian's immediate `pause`, permissionless `execute` with no
//! executor, and the controller's `supply` as the pause probe. The harness
//! owner holds every role, so one address plays proposer and guardian; the
//! roles are separate on mainnet. The MC-4 test was flipped by PR #196
//! (121de30, 6f70ab5): the guardian's `pause` advances the emergency epoch
//! every `Unpause` guard records, and a stale one is refused with
//! `EmergencyEpochMismatch` (57). The MC-5 test was flipped by the ordering
//! fix (gov_b G7): an `Unpause` is refused with `ControllerUpgradePending`
//! (61) while an `UpgradeController` is pending, and an executed upgrade
//! voids every reopen proposed before it.

use governance_interface::{AdminOperation, OperationState};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, LendingTest, ALICE};

/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`.
const GRACE: u32 = 120_960;
/// stellar-contract-utils pausable `ExpectedPause`: unpause needs a paused contract.
const EXPECTED_PAUSE: u32 = 1001;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
}

fn flatten<T, C>(
    r: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match r {
        Ok(_) => Ok(()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("unexpected InvokeError {e:?}"),
    }
}

fn propose(t: &LendingTest, op: &AdminOperation, byte: u8) -> BytesN<32> {
    t.gov_iface_client().propose(&t.admin(), op, &salt(t, byte))
}

fn op_state(t: &LendingTest, id: &BytesN<32>) -> OperationState {
    t.gov_iface_client().get_operation_state(id)
}

/// The guardian's immediate pause, through governance.
fn guardian_pause(t: &LendingTest) {
    t.gov_iface_client().pause(&t.admin());
}

/// Permissionless `execute` (no executor) against the controller.
fn exec_raw(
    t: &LendingTest,
    function: &str,
    args: Vec<Val>,
    predecessor: BytesN<32>,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    flatten(t.gov_iface_client().try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, function),
        &args,
        &predecessor,
        &salt(t, byte),
    ))
}

fn exec_unpause(t: &LendingTest, byte: u8) -> Result<(), soroban_sdk::Error> {
    exec_raw(t, "unpause", Vec::new(&t.env), salt(t, 0), byte)
}

fn upload_controller_wasm(env: &soroban_sdk::Env) -> BytesN<32> {
    let mut bytes = std::fs::read("target/wasm32v1-none/release/controller.wasm");
    if bytes.is_err() {
        bytes = std::fs::read("../../target/wasm32v1-none/release/controller.wasm");
    }
    let bytes = bytes.expect("controller WASM not found. Run 'make build' first.");
    env.deployer()
        .upload_contract_wasm(soroban_sdk::Bytes::from_slice(env, &bytes))
}

/// MC-4, `Unpause` arm, fixed. An `Unpause` proposed while the controller is
/// open cannot execute (1001) and the refusal does not consume it, so it stays
/// Ready for the whole grace window. The guardian pause moves the emergency
/// epoch its guard recorded: governance refuses it (57), the controller stays
/// paused, and reopening takes a fresh proposal that serves its full delay.
#[test]
fn mc4_unpause_queued_while_open_is_refused_after_guardian_pause() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);

    let delay = t.gov_iface_client().get_min_delay();
    let id = propose(&t, &AdminOperation::Unpause, 1);
    advance(&t, delay);
    assert_eq!(op_state(&t, &id), OperationState::Ready);

    // Open controller: the target refuses, the operation survives.
    assert_contract_error(exec_unpause(&t, 1), EXPECTED_PAUSE);
    assert_eq!(
        op_state(&t, &id),
        OperationState::Ready,
        "a refused execute rolls back the Done write"
    );

    // Last ledger of the grace window: still live.
    advance(&t, GRACE);
    assert_contract_error(exec_unpause(&t, 1), EXPECTED_PAUSE);

    guardian_pause(&t);
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);

    // Same ledger as the pause, no executor: refused by the emergency epoch.
    assert_contract_error(
        exec_unpause(&t, 1),
        GenericError::EmergencyEpochMismatch as u32,
    );
    assert_eq!(op_state(&t, &id), OperationState::Ready);
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);

    // Reopening is a fresh decision that serves its delay after the pause.
    let fresh = propose(&t, &AdminOperation::Unpause, 2);
    assert_contract_error(exec_unpause(&t, 2), errors::TIMELOCK_UNEXPECTED_STATE);
    advance(&t, delay);
    exec_unpause(&t, 2).expect("an Unpause proposed after the pause executes");
    assert_eq!(op_state(&t, &fresh), OperationState::Unset);
    t.try_supply(ALICE, "USDC", 100.0)
        .expect("controller reopened by the post-pause proposal");
}

/// MC-5, fixed (gov_b G7; was `mc5_unpause_runs_before_a_ready_controller_upgrade`,
/// which pinned the reopen running on the old code first). With the
/// controller paused for an incident, a fix `UpgradeController` and an
/// `Unpause` proposed together become Ready together. The `Unpause` is
/// refused while the fix is pending and the controller stays paused; the fix
/// executes and voids that early reopen (it predates the emergency action),
/// and a reopen proposed after the fix serves its delay and opens the
/// controller on the new code. Reopening on the fix therefore always costs
/// one proposal and one Standard delay after the upgrade executes.
#[test]
fn mc5_unpause_waits_for_the_pending_controller_upgrade() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    guardian_pause(&t);

    let hash = upload_controller_wasm(&t.env);
    let upgrade_args: Vec<Val> = vec![&t.env, hash.clone().into_val(&t.env)];
    let up_id = propose(&t, &AdminOperation::UpgradeController(hash), 2);
    let un_id = propose(&t, &AdminOperation::Unpause, 3);

    // Harness: Standard 50, Sensitive max(50, 12) = 50. Mainnet: both 12.
    let delay = t.gov_iface_client().get_min_delay().max(12);
    advance(&t, delay);
    assert_eq!(op_state(&t, &up_id), OperationState::Ready);
    assert_eq!(op_state(&t, &un_id), OperationState::Ready);

    // No opt-in ordering: `propose` only ever schedules predecessor = 0.
    assert_contract_error(
        exec_raw(&t, "unpause", Vec::new(&t.env), up_id.clone(), 3),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );

    assert_contract_error(
        exec_unpause(&t, 3),
        GenericError::ControllerUpgradePending as u32,
    );
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);
    assert_eq!(op_state(&t, &un_id), OperationState::Ready);

    exec_raw(&t, "upgrade", upgrade_args, salt(&t, 0), 2).expect("upgrade executes");
    assert_contract_error(t.try_supply(ALICE, "USDC", 100.0), errors::CONTRACT_PAUSED);
    assert_contract_error(
        exec_unpause(&t, 3),
        GenericError::EmergencyEpochMismatch as u32,
    );
    let again = propose(&t, &AdminOperation::Unpause, 4);
    assert_eq!(
        op_state(&t, &again),
        OperationState::Waiting,
        "reopening on the fix needs another full delay"
    );
    advance(&t, delay);
    exec_unpause(&t, 4).expect("the reopen proposed after the fix executes");
    t.try_supply(ALICE, "USDC", 100.0)
        .expect("controller reopened on the new code");
}
