//! Second opinion on Specula governance CR-1 and CR-6.
//!
//! CR-1: `TransferGovOwnership { new_owner: X, live_until_ledger: 0 }` is the
//! timelocked cancel of a pending nomination of X. `apply_transfer_ownership`
//! hands the zero deadline to `role_transfer::transfer_role`, which panics with
//! `NoPendingTransfer` (2200) when nothing is pending. The panic rolls back the
//! Done write that `set_execute_operation` made, so the cancel stays Ready for
//! its whole grace window and, because `execute_self` accepts `executor = None`,
//! anyone can run it the moment a later nomination of X is live. The window is
//! bounded by `TIMELOCK_OPERATION_GRACE_LEDGERS` after the cancel's ready ledger
//! and by the owner's own `cancel` of the stale id.
//!
//! CR-6: the only writers that clear `OperationLedger(id)` and the `RecoveryOp`
//! sidecar are `finish_execute` (needs a non-expired Ready op) and `cancel`
//! (refuses recovery ops). An expired canceller reset is therefore stuck under
//! its salt: not executable (40), not cancellable (46), not re-proposable with
//! the same salt (4000). A fresh salt schedules and executes normally.
//!
//! Every step uses production entry points: `propose` and
//! `propose_canceller_reset` as the owner, permissionless `execute_self` and
//! `execute_canceller_reset` with no executor, `cancel` and `accept_ownership`.
//! Both tests pin the HEAD behaviour and name the assertions that must flip
//! once a fix lands.

use governance_interface::{AdminOperation, OperationState, TransferOwnershipArgs};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Symbol, Vec};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, LendingTest};

/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`.
const GRACE: u32 = 120_960;
/// `governance::constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`.
const RECOVERY: u32 = 518_400;
/// OZ `TimelockError::OperationAlreadyScheduled`.
const ALREADY_SCHEDULED: u32 = 4000;
const EXPIRED: u32 = GenericError::TimelockOperationExpired as u32;
const NOT_CANCELLABLE: u32 = GenericError::OperationNotCancellable as u32;

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

/// The timelocked cancel of a pending nomination of `who`.
fn cancel_of(who: &Address) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: who.clone(),
        live_until_ledger: 0,
    })
}

/// A nomination of `who` that stays acceptable for `window` ledgers.
fn nomination_of(t: &LendingTest, who: &Address, window: u32) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: who.clone(),
        live_until_ledger: t.env.ledger().sequence() + window,
    })
}

fn exec_self(t: &LendingTest, op: &AdminOperation, s: u8) -> Result<(), soroban_sdk::Error> {
    flatten(
        t.gov_iface_client()
            .try_execute_self(&None, op, &salt(t, s)),
    )
}

/// Proposes `op` as the owner, waits the delay and executes it with no executor.
fn run_self_op(t: &LendingTest, op: &AdminOperation, s: u8) -> BytesN<32> {
    let gov = t.gov_iface_client();
    let id = gov.propose(&t.admin(), op, &salt(t, s));
    advance(t, gov.get_min_delay());
    exec_self(t, op, s).expect("owner op executes");
    id
}

/// CR-1 at HEAD. The owner queues a cancel of X while nothing is pending; it
/// reverts with 2200 and stays Ready. The owner then nominates X for real and
/// a stranger drives the stale cancel, which can only succeed because X is now
/// pending: X's `accept_ownership` reports 2200 and the owner has to nominate
/// again. The owner's `cancel` of the stale id is the in-protocol mitigation.
#[test]
fn cr1_reverted_zero_deadline_cancel_voids_a_later_nomination() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let owner = t.admin();
    let x = Address::generate(&t.env);
    let guardian = Symbol::new(&t.env, "GUARDIAN");

    // A cancel proposed while nothing is pending reverts and is not consumed.
    let stale = cancel_of(&x);
    let stale_id = gov.propose(&owner, &stale, &salt(&t, 1));
    advance(&t, d);
    assert_contract_error(exec_self(&t, &stale, 1), errors::NO_PENDING_TRANSFER);
    assert_eq!(
        gov.get_operation_state(&stale_id),
        OperationState::Ready,
        "the reverted cancel stays Ready (flips once a reverted cancel is cleared or bound)"
    );

    // The legitimate nomination lands.
    run_self_op(&t, &nomination_of(&t, &x, 10_000), 2);

    // Anyone runs the stale cancel; it succeeds only because X is pending.
    exec_self(&t, &stale, 1).expect("stale cancel executes against the live nomination");
    assert_eq!(gov.get_operation_state(&stale_id), OperationState::Unset);
    assert_contract_error(
        flatten(gov.try_accept_ownership()),
        errors::NO_PENDING_TRANSFER,
    );
    assert!(!gov.has_role(&x, &guardian), "X never became owner");

    // The owner nominates again, waits another delay, and X accepts.
    run_self_op(&t, &nomination_of(&t, &x, 10_000), 3);
    gov.accept_ownership();
    assert!(
        gov.has_role(&x, &guardian),
        "X is the owner after the repeat"
    );

    // Mitigation: a stale cancel is an ordinary op, the owner-canceller clears it.
    let second = cancel_of(&owner);
    let second_id = gov.propose(&x, &second, &salt(&t, 4));
    advance(&t, d);
    assert_contract_error(exec_self(&t, &second, 4), errors::NO_PENDING_TRANSFER);
    assert_eq!(gov.get_operation_state(&second_id), OperationState::Ready);
    gov.cancel(&x, &second_id);
    assert_eq!(gov.get_operation_state(&second_id), OperationState::Unset);
}

/// CR-1 bound. A stale cancel older than its grace window is refused with 40,
/// so a nomination made after `ready + GRACE` is safe from it and X accepts.
#[test]
fn cr1_stale_cancel_expires_with_the_grace_window() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let owner = t.admin();
    let x = Address::generate(&t.env);
    let guardian = Symbol::new(&t.env, "GUARDIAN");

    let stale = cancel_of(&x);
    let stale_id = gov.propose(&owner, &stale, &salt(&t, 5));
    advance(&t, d);
    assert_contract_error(exec_self(&t, &stale, 5), errors::NO_PENDING_TRANSFER);
    advance(&t, GRACE + 1);
    assert_eq!(gov.get_operation_state(&stale_id), OperationState::Ready);

    run_self_op(&t, &nomination_of(&t, &x, 10_000), 6);
    assert_contract_error(exec_self(&t, &stale, 5), EXPIRED);
    gov.accept_ownership();
    assert!(gov.has_role(&x, &guardian));
}

/// CR-6 at HEAD, recovery variant. An expired canceller reset is stuck under
/// its salt: execute 40, cancel 46, same-salt re-propose 4000. A fresh salt is
/// the only escape and the stuck id survives it.
#[test]
fn cr6_expired_recovery_reset_is_stuck_under_its_salt() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let fresh = Address::generate(&t.env);
    let set: Vec<Address> = vec![&t.env, fresh.clone()];
    let canceller = Symbol::new(&t.env, "CANCELLER");

    let id = gov.propose_canceller_reset(&set, &salt(&t, 7));
    advance(&t, RECOVERY + GRACE + 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    assert_contract_error(
        flatten(gov.try_execute_canceller_reset(&None, &set, &salt(&t, 7))),
        EXPIRED,
    );
    assert_contract_error(flatten(gov.try_cancel(&t.admin(), &id)), NOT_CANCELLABLE);
    assert_contract_error(
        flatten(gov.try_propose_canceller_reset(&set, &salt(&t, 7))),
        ALREADY_SCHEDULED,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        OperationState::Ready,
        "the expired recovery id is stuck (flips once propose clears expired ids)"
    );
    assert!(!gov.has_role(&fresh, &canceller));

    // A fresh salt schedules and executes; the stuck id is untouched.
    let id2 = gov.propose_canceller_reset(&set, &salt(&t, 8));
    assert_ne!(id2, id);
    assert_eq!(gov.get_operation_state(&id2), OperationState::Waiting);
    advance(&t, RECOVERY);
    gov.execute_canceller_reset(&None, &set, &salt(&t, 8));
    assert!(gov.has_role(&fresh, &canceller));
    assert_eq!(gov.get_operation_state(&id2), OperationState::Unset);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);
}
