//! GH-26. `executor = None` skips auth and the role check by design. Under
//! enforcing auth: anyone executes a ready operation, nobody executes one that
//! is not ready or has expired, and `Some(address)` needs that address's
//! signature and role.

use controller::types::PositionLimits;
use governance_interface::{AdminOperation, OperationState};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::xdr::{ScErrorCode, ScErrorType};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, LendingTest};

/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`, which is private
/// to the contract crate.
const GRACE: u32 = 120_960;

/// The harness timelock delay, read from the contract instead of restated.
fn delay(t: &LendingTest) -> u32 {
    t.gov_iface_client().get_min_delay()
}

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
    let d = delay(&t);
    let id = propose(&t, 1);
    t.env.ledger().with_mut(|l| l.sequence_number += d);
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
    let d = delay(&t);
    let id = propose(&t, 2);
    t.env.ledger().with_mut(|l| l.sequence_number += d);
    let stranger = Address::generate(&t.env);

    // Two distinct refusals, previously indistinguishable behind `is_err()`:
    // the role holder that did not sign is stopped by the host auth check, the
    // signer without the role by the contract's own role check.
    t.env.set_auths(&[]);
    assert_eq!(
        execute(&t, Some(t.admin()), 2).expect_err("the role holder must still sign"),
        soroban_sdk::Error::from_type_and_code(ScErrorType::Context, ScErrorCode::InvalidAction),
        "an unsigned named executor must be refused by the host auth check"
    );
    t.env.mock_all_auths_allowing_non_root_auth();
    assert_contract_error(execute(&t, Some(stranger), 2), errors::UNAUTHORIZED);

    // Neither refusal consumed the operation.
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Ready
    );
}

#[test]
fn nobody_executes_before_the_delay_or_after_the_grace_window() {
    let t = LendingTest::new().build();
    let d = delay(&t);
    let id = propose(&t, 3);
    assert_contract_error(execute(&t, None, 3), errors::TIMELOCK_UNEXPECTED_STATE);

    // `require_operation_not_expired` accepts `sequence() <= expires_at`
    // (timelock/mod.rs:92-97), so the last ledger of the grace window must
    // still execute. Without this leg a `<` for `<=` regression is invisible.
    t.env.ledger().with_mut(|l| l.sequence_number += d + GRACE);
    execute(&t, None, 3).expect("execution at exactly expires_at must succeed");
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Unset
    );

    // One ledger further out, a freshly scheduled operation is expired.
    let expired = propose(&t, 4);
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += d + GRACE + 1);
    assert_contract_error(
        execute(&t, None, 4),
        GenericError::TimelockOperationExpired as u32,
    );
    assert_eq!(
        t.gov_iface_client().get_operation_state(&expired),
        OperationState::Ready,
        "an expired operation stays scheduled; only its execution is refused"
    );
}
