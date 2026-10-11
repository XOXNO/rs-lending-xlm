//! Second opinion on Specula governance MC-2 and MC-3: an operation the owner
//! queued before an ownership handover still executes after the handover.
//! Nothing binds a scheduled operation to the owner that proposed it:
//! `propose` checks `proposer == owner` once, `prepare_execute` checks only the
//! optional executor, the hash and the grace window, and the self-op appliers
//! (`apply_transfer_ownership`, `apply_grant_role`, `apply_canceller_reset`)
//! read the *current* owner without comparing it to the proposer.
//!
//! Every step uses production entry points: `propose` and
//! `propose_canceller_reset` as the owner of the moment, permissionless
//! `execute_self` / `execute_canceller_reset` with no executor,
//! `accept_ownership` by the pending owner, `cancel` and `pause`.
//!
//! Flipped by PR #196 (ff3cdca, 8ca2d7b, 6f70ab5): `accept_ownership` advances
//! the owner epoch, the operation guard records it for every owner-only kind
//! and for the canceller reset, and an execute under a later epoch is refused
//! with `NotAuthorized` (44).

use governance_interface::{AdminOperation, OperationState, RoleArgs, TransferOwnershipArgs};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Symbol, Vec};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, usdc_preset, LendingTest, ALICE};

/// `governance::constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`.
const RECOVERY: u32 = 518_400;
const NOT_CANCELLABLE: u32 = GenericError::OperationNotCancellable as u32;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn sym(t: &LendingTest, s: &str) -> Symbol {
    Symbol::new(&t.env, s)
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

fn transfer_to(t: &LendingTest, who: &Address) -> AdminOperation {
    AdminOperation::TransferGovOwnership(TransferOwnershipArgs {
        new_owner: who.clone(),
        live_until_ledger: t.env.ledger().sequence() + 10_000,
    })
}

fn grant(t: &LendingTest, who: &Address, role: &str) -> AdminOperation {
    AdminOperation::GrantGovRole(RoleArgs {
        account: who.clone(),
        role: sym(t, role),
    })
}

/// Proposes `op` as `owner`, waits the Sensitive delay (the harness minimum,
/// 50, is above the 12-ledger floor) and executes it with no executor.
fn run_self_op(t: &LendingTest, owner: &Address, op: &AdminOperation, s: u8) {
    let gov = t.gov_iface_client();
    gov.propose(owner, op, &salt(t, s));
    advance(t, gov.get_min_delay());
    gov.execute_self(&None, op, &salt(t, s));
}

/// MC-2 fixed. Owner A queues, while it is still the owner, a second
/// `TransferGovOwnership` to its alternate key C and two `GrantGovRole`s to
/// itself, then hands over to B. After `accept_ownership` strips A of every
/// role, the leftovers are Ready but every execute path refuses them: A stays
/// unarmed, C cannot accept anything, B keeps ownership and every role, and B
/// clears the dead queue with the CANCELLER seat the handover gave it.
#[test]
fn mc2_leftover_owner_ops_are_refused_after_handover() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    let _ = t.get_or_create_user(ALICE);
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let c = Address::generate(&t.env);
    let roles = ["ORACLE", "PROPOSER", "EXECUTOR", "CANCELLER", "GUARDIAN"];

    // A queues the honest handover and three leftovers in the same window.
    let to_b = transfer_to(&t, &b);
    let to_c = transfer_to(&t, &c);
    let rearm_canceller = grant(&t, &a, "CANCELLER");
    let rearm_guardian = grant(&t, &a, "GUARDIAN");
    gov.propose(&a, &to_b, &salt(&t, 1));
    let id_c = gov.propose(&a, &to_c, &salt(&t, 2));
    let id_canc = gov.propose(&a, &rearm_canceller, &salt(&t, 3));
    let id_guard = gov.propose(&a, &rearm_guardian, &salt(&t, 4));
    advance(&t, d);

    // B cannot clear the queue before accepting: it holds no CANCELLER yet.
    assert_contract_error(flatten(gov.try_cancel(&b, &id_c)), errors::UNAUTHORIZED);

    gov.execute_self(&None, &to_b, &salt(&t, 1));
    gov.accept_ownership();
    for role in roles {
        assert!(gov.has_role(&b, &sym(&t, role)), "B holds {role}");
        assert!(!gov.has_role(&a, &sym(&t, role)), "A lost {role}");
    }
    // The leftovers survived the handover, all Ready.
    for id in [&id_c, &id_canc, &id_guard] {
        assert_eq!(gov.get_operation_state(id), OperationState::Ready);
    }

    // (a) The leftover grants carry A's owner epoch; the handover moved it, so
    // both are refused and A stays unarmed.
    assert_contract_error(
        flatten(gov.try_execute_self(&None, &rearm_canceller, &salt(&t, 3))),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        flatten(gov.try_execute_self(&None, &rearm_guardian, &salt(&t, 4))),
        errors::NOT_AUTHORIZED,
    );
    assert!(!gov.has_role(&a, &sym(&t, "CANCELLER")));
    assert!(!gov.has_role(&a, &sym(&t, "GUARDIAN")));

    // A can neither veto a proposal of the new owner ...
    let upd = AdminOperation::UpdateGovDelay(d + 10);
    let id_upd = gov.propose(&b, &upd, &salt(&t, 5));
    assert_contract_error(flatten(gov.try_cancel(&a, &id_upd)), errors::UNAUTHORIZED);
    assert_eq!(gov.get_operation_state(&id_upd), OperationState::Waiting);
    // ... nor halt lending.
    assert_contract_error(flatten(gov.try_pause(&a)), errors::UNAUTHORIZED);
    t.try_supply(ALICE, "USDC", 1.0)
        .expect("lending stays open");

    // (b) The leftover transfer is refused too: nothing is pending for C, B
    // keeps ownership and every role, and B's own proposals go through.
    let gov = t.gov_iface_client();
    assert_contract_error(
        flatten(gov.try_execute_self(&None, &to_c, &salt(&t, 2))),
        errors::NOT_AUTHORIZED,
    );
    assert_contract_error(
        flatten(gov.try_accept_ownership()),
        errors::NO_PENDING_TRANSFER,
    );
    for role in roles {
        assert!(gov.has_role(&b, &sym(&t, role)), "B keeps {role}");
        assert!(!gov.has_role(&c, &sym(&t, role)), "C never held {role}");
    }
    gov.propose(&b, &upd, &salt(&t, 6));

    // B clears the dead queue; the leftovers are Ready until then.
    for id in [&id_c, &id_canc, &id_guard] {
        assert_eq!(gov.get_operation_state(id), OperationState::Ready);
        gov.cancel(&b, id);
        assert_eq!(gov.get_operation_state(id), OperationState::Unset);
    }
}

/// MC-3 fixed. Owner A schedules a canceller reset to its own council
/// {A2, A3}, hands over to B, and B seats its own canceller B2. When the
/// ~30-day Recovery delay elapses A's reset is Ready but refused: it carries
/// A's owner epoch. B's interim canceller keeps its seat and A's council never
/// gets one. B's own counter-reset is the usual uncancellable recovery
/// operation and lands after its own full delay.
#[test]
fn mc3_old_owner_canceller_reset_is_refused_after_handover() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let a2 = Address::generate(&t.env);
    let a3 = Address::generate(&t.env);
    let b2 = Address::generate(&t.env);
    let b3 = Address::generate(&t.env);
    let canceller = sym(&t, "CANCELLER");
    let a_council: Vec<Address> = vec![&t.env, a2.clone(), a3.clone()];
    // B's reset seats a fresh member: re-listing a current holder revokes and
    // re-grants it in one call, which the debug test host's rent accounting
    // cannot meter (the harness minimum TTL is 10 ledgers).
    let b_council: Vec<Address> = vec![&t.env, b3.clone()];

    // A, still the owner, schedules its council. Ready at 100 + RECOVERY.
    let reset_id = gov.propose_canceller_reset(&a_council, &salt(&t, 7));
    run_self_op(&t, &a, &transfer_to(&t, &b), 1);
    gov.accept_ownership();
    assert!(gov.has_role(&b, &canceller));
    assert!(!gov.has_role(&a, &canceller));

    // B seats its own canceller and queues a counter-reset at once.
    run_self_op(&t, &b, &grant(&t, &b2, "CANCELLER"), 8);
    assert!(gov.has_role(&b2, &canceller));
    let counter_id = gov.propose_canceller_reset(&b_council, &salt(&t, 9));

    // B's own live reset is uncancellable, for the owner and its canceller.
    assert_contract_error(flatten(gov.try_cancel(&b, &counter_id)), NOT_CANCELLABLE);
    assert_contract_error(flatten(gov.try_cancel(&b2, &counter_id)), NOT_CANCELLABLE);

    // A's reset is Ready one Sensitive delay before B's counter-reset.
    advance(&t, RECOVERY - d);
    assert_eq!(gov.get_operation_state(&reset_id), OperationState::Ready);
    assert_eq!(
        gov.get_operation_state(&counter_id),
        OperationState::Waiting
    );

    // Refused for anyone: it carries A's owner epoch. Nobody is re-seated.
    assert_contract_error(
        flatten(gov.try_execute_canceller_reset(&None, &a_council, &salt(&t, 7))),
        errors::NOT_AUTHORIZED,
    );
    assert!(!gov.has_role(&a2, &canceller));
    assert!(!gov.has_role(&a3, &canceller));
    assert!(gov.has_role(&b2, &canceller));
    assert!(gov.has_role(&b, &canceller), "owner keeps its own seat");
    assert_eq!(gov.get_operation_state(&reset_id), OperationState::Ready);

    // A2 and A3 hold nothing, so they cannot veto B's proposals.
    let upgrade = AdminOperation::UpgradeGov(BytesN::<32>::from_array(&t.env, &[9u8; 32]));
    let id_up = gov.propose(&b, &upgrade, &salt(&t, 11));
    assert_contract_error(flatten(gov.try_cancel(&a2, &id_up)), errors::UNAUTHORIZED);
    assert_eq!(gov.get_operation_state(&id_up), OperationState::Waiting);

    // B's counter-reset lands after its own delay and seats B's council.
    advance(&t, d);
    gov.execute_canceller_reset(&None, &b_council, &salt(&t, 9));
    assert!(gov.has_role(&b3, &canceller));
    assert!(!gov.has_role(&b2, &canceller));
    assert!(!gov.has_role(&a2, &canceller));
    assert!(!gov.has_role(&a3, &canceller));
    assert!(gov.has_role(&b, &canceller), "owner keeps its own seat");
}
