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
//! Both tests pin the HEAD behaviour and name the assertions that must flip
//! once an owner-epoch (or proposing-owner stamp) guard exists in
//! `prepare_execute`.

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

/// MC-2 at HEAD. Owner A queues, while it is still the owner, a second
/// `TransferGovOwnership` to its alternate key C and two `GrantGovRole`s to
/// itself, then hands over to B. After `accept_ownership` strips A of every
/// role, anyone executes the leftovers: A is re-armed as CANCELLER (vetoes
/// B's proposals) and GUARDIAN (pauses the controller), and the leftover
/// transfer hands governance to C, so B loses ownership and all roles.
///
/// B's only mitigation is `cancel`, which it can use only after accepting
/// (it holds no CANCELLER before) and only before someone executes the
/// already-Ready leftover.
#[test]
fn mc2_leftover_owner_ops_execute_after_handover() {
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

    // (a) Leftover grants re-arm the former owner. The separation check runs
    // against the current owner B, and A holds no EXECUTOR, so both pass.
    // FLIP after fix: both must fail with NotAuthorized (44) and leave A
    // without the role.
    gov.execute_self(&None, &rearm_canceller, &salt(&t, 3));
    gov.execute_self(&None, &rearm_guardian, &salt(&t, 4));
    assert!(gov.has_role(&a, &sym(&t, "CANCELLER")));
    assert!(gov.has_role(&a, &sym(&t, "GUARDIAN")));

    // A vetoes a proposal of the new owner ...
    let upd = AdminOperation::UpdateGovDelay(d + 10);
    let id_upd = gov.propose(&b, &upd, &salt(&t, 5));
    gov.cancel(&a, &id_upd);
    assert_eq!(gov.get_operation_state(&id_upd), OperationState::Unset);
    // ... and halts lending with the immediate guardian power.
    gov.pause(&a);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0).map(|_| ()),
        errors::CONTRACT_PAUSED,
    );

    // (b) The leftover transfer hijacks the handover. `apply_transfer_ownership`
    // overwrites the pending owner with C and C accepts.
    // FLIP after fix: execute_self must fail with NotAuthorized (44), B keeps
    // ownership and PROPOSER.
    let gov = t.gov_iface_client();
    gov.execute_self(&None, &to_c, &salt(&t, 2));
    gov.accept_ownership();
    for role in roles {
        assert!(gov.has_role(&c, &sym(&t, role)), "C holds {role}");
        assert!(!gov.has_role(&b, &sym(&t, role)), "B lost {role}");
    }
    assert_contract_error(
        flatten(gov.try_propose(&b, &upd, &salt(&t, 6))),
        errors::UNAUTHORIZED,
    );
    // A's re-armed roles survive the second handover too.
    assert!(gov.has_role(&a, &sym(&t, "CANCELLER")));
}

/// MC-3 at HEAD. Owner A schedules a canceller reset to its own council
/// {A2, A3}, hands over to B, and B seats its own canceller B2. B cannot
/// cancel A's reset (recovery ops are uncancellable for everyone), and when
/// the ~30-day Recovery delay elapses anyone executes it: B2 is revoked, A2
/// and A3 are seated, and they veto B's revocation of A2 and B's upgrade.
/// B's own counter-reset, proposed right after accepting, is still Waiting
/// then; it only lands after its own full Recovery delay.
#[test]
fn mc3_old_owner_canceller_reset_survives_handover() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let a = t.admin();
    let b = Address::generate(&t.env);
    let a2 = Address::generate(&t.env);
    let a3 = Address::generate(&t.env);
    let b2 = Address::generate(&t.env);
    let canceller = sym(&t, "CANCELLER");
    let a_council: Vec<Address> = vec![&t.env, a2.clone(), a3.clone()];
    let b_council: Vec<Address> = vec![&t.env, b2.clone()];

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

    // Neither the new owner nor its canceller can cancel A's reset.
    assert_contract_error(flatten(gov.try_cancel(&b, &reset_id)), NOT_CANCELLABLE);
    assert_contract_error(flatten(gov.try_cancel(&b2, &reset_id)), NOT_CANCELLABLE);

    // A's reset is Ready one Sensitive delay before B's counter-reset.
    advance(&t, RECOVERY - d);
    assert_eq!(gov.get_operation_state(&reset_id), OperationState::Ready);
    assert_eq!(
        gov.get_operation_state(&counter_id),
        OperationState::Waiting
    );

    // Anyone executes it. `apply_canceller_reset` keeps only the current
    // owner B, revokes B2 and seats A2 and A3.
    // FLIP after fix: must fail with NotAuthorized (44); B2 keeps the role.
    gov.execute_canceller_reset(&None, &a_council, &salt(&t, 7));
    assert!(gov.has_role(&a2, &canceller));
    assert!(gov.has_role(&a3, &canceller));
    assert!(!gov.has_role(&b2, &canceller));
    assert!(gov.has_role(&b, &canceller), "owner keeps its own seat");

    // A's council vetoes B. A2 cannot cancel its own revocation, A3 can.
    let revoke_a2 = AdminOperation::RevokeGovRole(RoleArgs {
        account: a2.clone(),
        role: canceller.clone(),
    });
    let id_rev = gov.propose(&b, &revoke_a2, &salt(&t, 10));
    assert_contract_error(flatten(gov.try_cancel(&a2, &id_rev)), NOT_CANCELLABLE);
    gov.cancel(&a3, &id_rev);
    assert_eq!(gov.get_operation_state(&id_rev), OperationState::Unset);
    let upgrade = AdminOperation::UpgradeGov(BytesN::<32>::from_array(&t.env, &[9u8; 32]));
    let id_up = gov.propose(&b, &upgrade, &salt(&t, 11));
    gov.cancel(&a2, &id_up);
    assert_eq!(gov.get_operation_state(&id_up), OperationState::Unset);

    // B's counter-reset lands after its own delay and restores B's council.
    advance(&t, d);
    gov.execute_canceller_reset(&None, &b_council, &salt(&t, 9));
    assert!(gov.has_role(&b2, &canceller));
    assert!(!gov.has_role(&a2, &canceller));
    assert!(!gov.has_role(&a3, &canceller));
}
