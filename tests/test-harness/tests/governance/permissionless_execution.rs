//! GH-26. `executor = None` skips auth and the role check by design. Under
//! enforcing auth: anyone executes a ready operation, nobody executes one that
//! is not ready or has expired, and `Some(address)` needs that address's
//! signature and role.

use controller::types::PositionLimits;
use governance_interface::{AdminOperation, OperationState};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol};
use test_harness::LendingTest;

const DELAY: u32 = 50;
const GRACE: u32 = 120_960;

fn salt(env: &soroban_sdk::Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn limits() -> PositionLimits {
    PositionLimits {
        max_supply_positions: 4,
        max_borrow_positions: 3,
    }
}

fn propose(t: &LendingTest, byte: u8) -> BytesN<32> {
    t.gov_iface_client().propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(limits()),
        &salt(&t.env, byte),
    )
}

fn execute(t: &LendingTest, executor: Option<Address>, byte: u8) -> Result<(), soroban_sdk::Error> {
    let result = t.gov_iface_client().try_execute(
        &executor,
        &t.controller,
        &Symbol::new(&t.env, "set_position_limits"),
        &soroban_sdk::vec![&t.env, limits().into_val(&t.env)],
        &salt(&t.env, 0),
        &salt(&t.env, byte),
    );
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    }
}

#[test]
fn anyone_executes_a_ready_operation_with_no_signature() {
    let t = LendingTest::new().build();
    let id = propose(&t, 1);
    t.env.ledger().with_mut(|l| l.sequence_number += DELAY);
    t.env.set_auths(&[]);
    execute(&t, None, 1).expect("no auth is required with executor = None");
    t.env.mock_all_auths_allowing_non_root_auth();
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Unset
    );
}

#[test]
fn a_named_executor_needs_its_signature_and_role() {
    let t = LendingTest::new().build();
    propose(&t, 2);
    t.env.ledger().with_mut(|l| l.sequence_number += DELAY);
    let stranger = Address::generate(&t.env);
    t.env.set_auths(&[]);
    assert!(
        execute(&t, Some(stranger), 2).is_err(),
        "no signature, no role"
    );
    assert!(
        execute(&t, Some(t.admin()), 2).is_err(),
        "the role holder still needs to sign"
    );
    t.env.mock_all_auths_allowing_non_root_auth();
}

#[test]
fn nobody_executes_before_the_delay_or_after_the_grace_window() {
    let t = LendingTest::new().build();
    propose(&t, 3);
    assert!(execute(&t, None, 3).is_err(), "not ready");
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += DELAY + GRACE + 1);
    assert!(execute(&t, None, 3).is_err(), "expired");
}
