//! Coverage for `controller/src/access.rs` ownership + role lifecycle.
//!
//! The constructor path is exercised by every `LendingTest::new()`. The
//! remaining surface (transfer/accept ownership, grant/revoke role,
//! pause/unpause, upgrade-with-pause) needs dedicated tests because the
//! existing suite only touches grant/revoke and pause/unpause obliquely.

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};
use test_harness::{usdc_preset, LendingTest, ALICE, BOB};

fn fresh() -> LendingTest {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    // Pre-register ALICE / BOB so `users.get(...)` is non-empty in tests
    // that grant/revoke roles or transfer ownership.
    let _ = t.get_or_create_user(ALICE);
    let _ = t.get_or_create_user(BOB);
    t
}

// ---------------------------------------------------------------------------
// transfer_ownership / accept_ownership — two-phase ownership transfer
// ---------------------------------------------------------------------------

#[test]
fn test_transfer_and_accept_ownership_completes() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let admin = t.admin();
    let new_owner = t.users.get(ALICE).unwrap().address.clone();

    // Phase 1: current admin proposes a new owner with a non-zero TTL.
    let ledger_seq = t.env.ledger().sequence();
    ctrl.transfer_ownership(&new_owner, &(ledger_seq + 1000));

    // Phase 2: candidate accepts. The hook then mirrors the admin slot
    // and grants the three operational roles to the new owner.
    t.env.mock_all_auths();
    ctrl.accept_ownership();

    // KEEPER / REVENUE / ORACLE all granted to the new owner.
    for role in ["KEEPER", "REVENUE", "ORACLE"] {
        let role_sym = Symbol::new(&t.env, role);
        assert!(
            ctrl.has_role(&new_owner, &role_sym),
            "{} should be granted to the new owner",
            role
        );
    }

    // Previous admin no longer holds KEEPER (only KEEPER was granted at
    // construction time so this is the only role to revoke).
    let keeper = Symbol::new(&t.env, "KEEPER");
    assert!(!ctrl.has_role(&admin, &keeper));
}

#[test]
fn test_transfer_ownership_with_zero_ttl_cancels_pending() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let candidate = t.users.get(ALICE).unwrap().address.clone();

    let ledger_seq = t.env.ledger().sequence();
    // Propose first…
    ctrl.transfer_ownership(&candidate, &(ledger_seq + 500));
    // …then cancel by passing 0 — exercises the `live_until_ledger == 0`
    // branch of `sync_pending_admin_transfer`.
    ctrl.transfer_ownership(&candidate, &0u32);
}

#[test]
fn test_transfer_ownership_to_self_keeps_roles() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    let admin = t.admin();

    // Self-transfer: previous_owner == new_owner exercises the
    // role-retention branch in `sync_owner_access_control`.
    let ledger_seq = t.env.ledger().sequence();
    ctrl.transfer_ownership(&admin, &(ledger_seq + 1000));
    t.env.mock_all_auths();
    ctrl.accept_ownership();

    let keeper = Symbol::new(&t.env, "KEEPER");
    assert!(ctrl.has_role(&admin, &keeper));
}

// ---------------------------------------------------------------------------
// grant_role / revoke_role — owner-gated
// ---------------------------------------------------------------------------

#[test]
fn test_grant_role_then_revoke_round_trip() {
    let t = fresh();
    let bob_addr = t.users.get(BOB).unwrap().address.clone();
    let oracle_sym = Symbol::new(&t.env, "ORACLE");

    assert!(!t.ctrl_client().has_role(&bob_addr, &oracle_sym));
    t.grant_role(BOB, "ORACLE");
    assert!(t.ctrl_client().has_role(&bob_addr, &oracle_sym));
    t.revoke_role(BOB, "ORACLE");
    assert!(!t.ctrl_client().has_role(&bob_addr, &oracle_sym));
}

#[test]
fn test_has_role_false_for_unknown_address() {
    let t = fresh();
    let stranger = Address::generate(&t.env);
    let keeper = Symbol::new(&t.env, "KEEPER");
    assert!(!t.ctrl_client().has_role(&stranger, &keeper));
}

// ---------------------------------------------------------------------------
// pause / unpause — owner-gated
// ---------------------------------------------------------------------------

#[test]
fn test_pause_unpause_round_trip() {
    let t = fresh();
    // `LendingTest::build` already unpauses after construction. Pause →
    // unpause exercises both endpoints from a clean state.
    t.pause();
    t.unpause();
}

// ---------------------------------------------------------------------------
// app_version + migrate
// ---------------------------------------------------------------------------

#[test]
fn test_app_version_defaults_to_initial() {
    let t = fresh();
    assert_eq!(t.ctrl_client().app_version(), 1);
}

#[test]
fn test_migrate_bumps_version_when_strictly_greater() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    ctrl.migrate(&2);
    assert_eq!(ctrl.app_version(), 2);
    ctrl.migrate(&5);
    assert_eq!(ctrl.app_version(), 5);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_migrate_rejects_equal_version() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    // Initial AppVersion is 1; calling migrate(1) must reject.
    ctrl.migrate(&1);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_migrate_rejects_lower_version() {
    let t = fresh();
    let ctrl = t.ctrl_client();
    ctrl.migrate(&3);
    // Downgrade attempt must reject.
    ctrl.migrate(&2);
}
