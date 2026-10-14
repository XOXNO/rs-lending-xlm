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
//!
//! Flipped by PR #196. CR-1 (e5790f1, 6f70ab5): the cancel's guard records the
//! nomination epoch at proposal, and the cancel changes the pending owner only
//! while that nomination is still the live one; otherwise it completes as a
//! no-op and is consumed. CR-6 (94beb7b): a same-salt proposal clears an
//! expired operation, recovery resets included, and schedules it again.

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

/// Runs `f` with the minimum entry TTL raised to the maximum. A same-salt
/// re-proposal deletes the expired operation's ledger entries and writes them
/// again in one call; the test host's debug invocation metering
/// (soroban-env-host 28.0.2, `invocation_metering.rs`) subtracts the old
/// live-until ledger from the new one and overflows when a re-created entry
/// lives shorter than the entry it replaced, which the harness's 10-ledger
/// minimum TTL allows. The network does not run that code.
fn with_long_min_ttl<R>(t: &LendingTest, f: impl FnOnce() -> R) -> R {
    let info = t.env.ledger().get();
    let (persistent, temporary) = (info.min_persistent_entry_ttl, info.min_temp_entry_ttl);
    t.env.ledger().with_mut(|l| {
        l.min_persistent_entry_ttl = info.max_entry_ttl;
        l.min_temp_entry_ttl = info.max_entry_ttl;
    });
    let out = f();
    t.env.ledger().with_mut(|l| {
        l.min_persistent_entry_ttl = persistent;
        l.min_temp_entry_ttl = temporary;
    });
    out
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

/// CR-1, fixed. The owner queues a cancel of X while nothing is pending. Its
/// first execution completes as a no-op and consumes it (before PR #196 it
/// reverted with 2200 and stayed Ready). The owner then nominates X for real
/// and the consumed cancel cannot be run against it: X accepts.
#[test]
fn cr1_reverted_zero_deadline_cancel_cannot_void_a_later_nomination() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let owner = t.admin();
    let x = Address::generate(&t.env);
    let guardian = Symbol::new(&t.env, "GUARDIAN");

    // A cancel proposed while nothing is pending is consumed as a no-op.
    let stale = cancel_of(&x);
    let stale_id = gov.propose(&owner, &stale, &salt(&t, 1));
    advance(&t, d);
    exec_self(&t, &stale, 1).expect("a cancel with nothing pending completes as a no-op");
    assert_eq!(
        gov.get_operation_state(&stale_id),
        OperationState::Unset,
        "the no-op cancel is consumed"
    );
    assert_contract_error(
        flatten(gov.try_accept_ownership()),
        errors::NO_PENDING_TRANSFER,
    );

    // The legitimate nomination lands, and the consumed cancel cannot run.
    run_self_op(&t, &nomination_of(&t, &x, 10_000), 2);
    assert_contract_error(exec_self(&t, &stale, 1), errors::TIMELOCK_UNEXPECTED_STATE);
    gov.accept_ownership();
    assert!(gov.has_role(&x, &guardian), "X is the owner");

    // The same holds for the new owner: a cancel with nothing pending is
    // consumed without effect.
    let second = cancel_of(&owner);
    let second_id = gov.propose(&x, &second, &salt(&t, 4));
    advance(&t, d);
    exec_self(&t, &second, 4).expect("no-op cancel");
    assert_eq!(gov.get_operation_state(&second_id), OperationState::Unset);
    assert!(gov.has_role(&x, &guardian), "X stays the owner");
}

/// CR-1 bound, idle shape. A cancel queued before the nomination and never run
/// is refused once its grace window ends (40), so a nomination made after
/// `ready + GRACE` is safe from it and X accepts. Before PR #196 the first run
/// reverted with 2200 and left it Ready; since then a run would consume it as
/// a no-op, so this test leaves it idle.
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
    advance(&t, GRACE + 1);
    assert_eq!(gov.get_operation_state(&stale_id), OperationState::Ready);

    run_self_op(&t, &nomination_of(&t, &x, 10_000), 6);
    assert_contract_error(exec_self(&t, &stale, 5), EXPIRED);
    gov.accept_ownership();
    assert!(gov.has_role(&x, &guardian));
}

/// CR-6, recovery variant, fixed by PR #196 (94beb7b). An expired canceller
/// reset is refused (40) and stays uncancellable (46), but a same-salt
/// proposal clears it and schedules it again under the same id, which then
/// executes after its own delay. Before the fix that proposal failed with
/// 4000 and the id was stuck.
#[test]
fn cr6_expired_recovery_reset_is_replaced_by_a_same_salt_proposal() {
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
    assert!(!gov.has_role(&fresh, &canceller));

    // The same salt schedules again under the same id.
    let again = with_long_min_ttl(&t, || gov.propose_canceller_reset(&set, &salt(&t, 7)));
    assert_eq!(again, id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    assert_contract_error(
        flatten(gov.try_propose_canceller_reset(&set, &salt(&t, 7))),
        ALREADY_SCHEDULED,
    );
    advance(&t, RECOVERY);
    gov.execute_canceller_reset(&None, &set, &salt(&t, 7));
    assert!(gov.has_role(&fresh, &canceller));
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}
