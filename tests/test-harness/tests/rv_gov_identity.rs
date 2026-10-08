//! RV break pass: governance operation identity and timelock state. Every
//! test pins the SAFE expectation with the exact error code so it fails if the
//! attack works.

use controller::types::PositionLimits;
use governance_interface::{AdminOperation, OperationState, RoleArgs};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, LendingTest};

/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`.
const GRACE: u32 = 120_960;
/// `governance::constants::TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS`.
const RECOVERY: u32 = 518_400;
/// OZ `TimelockError::OperationAlreadyScheduled`.
const ALREADY_SCHEDULED: u32 = 4000;

fn salt(env: &soroban_sdk::Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn limits(supply: u32, borrow: u32) -> PositionLimits {
    PositionLimits {
        max_supply_positions: supply,
        max_borrow_positions: borrow,
    }
}

fn limits_args(t: &LendingTest, l: &PositionLimits) -> Vec<Val> {
    soroban_sdk::vec![&t.env, l.clone().into_val(&t.env)]
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

fn exec_raw(
    t: &LendingTest,
    target: &Address,
    function: &str,
    args: Vec<Val>,
    predecessor: BytesN<32>,
    s: BytesN<32>,
) -> Result<(), soroban_sdk::Error> {
    flatten(t.gov_iface_client().try_execute(
        &None,
        target,
        &Symbol::new(&t.env, function),
        &args,
        &predecessor,
        &s,
    ))
}

fn exec_self(
    t: &LendingTest,
    op: &AdminOperation,
    s: BytesN<32>,
) -> Result<(), soroban_sdk::Error> {
    flatten(t.gov_iface_client().try_execute_self(&None, op, &s))
}

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
}

/// G-1 replay: after `finish_execute` the id is Unset, a second execute with
/// the identical tuple is refused (4002) and the controller keeps the first
/// value; a fresh propose with the same (op, salt) is allowed and Waiting.
#[test]
fn replay_after_finish_execute_is_refused() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let l = limits(4, 3);
    let s = salt(&t.env, 1);
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &s,
    );
    advance(&t, d);
    exec_raw(
        &t,
        &t.controller,
        "set_position_limits",
        limits_args(&t, &l),
        salt(&t.env, 0),
        s.clone(),
    )
    .expect("first execution");
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);

    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args(&t, &l),
            salt(&t.env, 0),
            s.clone(),
        ),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );

    let id2 = gov.propose(&t.admin(), &AdminOperation::SetPositionLimits(l), &s);
    assert_eq!(id2, id, "same tuple hashes to the same id");
    assert_eq!(gov.get_operation_state(&id2), OperationState::Waiting);
}

/// G-2: the hash binds target, function, args and predecessor. Any tampered
/// tuple is Unset (4002) and the original still executes afterwards.
#[test]
fn hash_binds_function_args_and_predecessor() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let l = limits(4, 3);
    let s = salt(&t.env, 2);
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &s,
    );
    advance(&t, d);

    // tampered args
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args(&t, &limits(4, 4)),
            salt(&t.env, 0),
            s.clone(),
        ),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    // tampered function with the original args
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_min_borrow_collateral_usd",
            limits_args(&t, &l),
            salt(&t.env, 0),
            s.clone(),
        ),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    // non-zero predecessor (never schedulable through propose)
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args(&t, &l),
            salt(&t.env, 9),
            s.clone(),
        ),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    // semantically equal but differently typed arg: u32 limits encoded as a plain u32 Val
    let typed: Vec<Val> = soroban_sdk::vec![&t.env, 4u32.into_val(&t.env), 3u32.into_val(&t.env)];
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            typed,
            salt(&t.env, 0),
            s.clone(),
        ),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        OperationState::Ready,
        "no refusal consumed the op"
    );

    exec_raw(
        &t,
        &t.controller,
        "set_position_limits",
        limits_args(&t, &l),
        salt(&t.env, 0),
        s,
    )
    .expect("original tuple executes");
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

/// G-1 target confusion: a self op cannot be driven through generic `execute`
/// (InternalError 34) and a controller op cannot be driven through
/// `execute_self` (34). The self op then executes through its own path.
#[test]
fn self_and_external_paths_are_not_interchangeable() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let guardian = Symbol::new(&t.env, "GUARDIAN");
    let x = Address::generate(&t.env);
    let grant = AdminOperation::GrantGovRole(RoleArgs {
        account: x.clone(),
        role: guardian.clone(),
    });
    let s_grant = salt(&t.env, 3);
    let grant_id = gov.propose(&t.admin(), &grant, &s_grant);
    let l = limits(4, 3);
    let s_lim = salt(&t.env, 4);
    let lim_id = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &s_lim,
    );
    advance(&t, d);

    // generic execute against governance itself with the exact resolved tuple
    let grant_args: Vec<Val> = soroban_sdk::vec![
        &t.env,
        x.clone().into_val(&t.env),
        guardian.clone().into_val(&t.env)
    ];
    assert_contract_error(
        exec_raw(
            &t,
            &t.governance,
            "grant_role",
            grant_args,
            salt(&t.env, 0),
            s_grant.clone(),
        ),
        errors::codes::INTERNAL_ERROR,
    );
    assert!(!gov.has_role(&x, &guardian));
    // execute_self for a controller-targeting op
    assert_contract_error(
        exec_self(&t, &AdminOperation::SetPositionLimits(l), s_lim),
        errors::codes::INTERNAL_ERROR,
    );
    assert_eq!(gov.get_operation_state(&grant_id), OperationState::Ready);
    assert_eq!(gov.get_operation_state(&lim_id), OperationState::Ready);

    exec_self(&t, &grant, s_grant).expect("self path executes the grant");
    assert!(gov.has_role(&x, &guardian));
    assert_eq!(gov.get_operation_state(&grant_id), OperationState::Unset);
}

/// G-8 expiry residue: an expired op reads Ready, execution is refused (40),
/// the same (op, salt) cannot be re-proposed (4000) until a canceller clears it,
/// but a different salt schedules immediately, so no time-critical repeat is
/// blocked.
#[test]
fn expired_operation_residue_only_blocks_the_same_salt() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    let l = limits(4, 3);
    let s = salt(&t.env, 5);
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &s,
    );
    advance(&t, d + GRACE + 1);
    assert_contract_error(
        exec_raw(
            &t,
            &t.controller,
            "set_position_limits",
            limits_args(&t, &l),
            salt(&t.env, 0),
            s.clone(),
        ),
        GenericError::TimelockOperationExpired as u32,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        OperationState::Ready,
        "expired op stays scheduled"
    );

    assert_contract_error(
        flatten(gov.try_propose(
            &t.admin(),
            &AdminOperation::SetPositionLimits(l.clone()),
            &s,
        )),
        ALREADY_SCHEDULED,
    );
    // a different salt is not blocked
    let other = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &salt(&t.env, 6),
    );
    assert_eq!(gov.get_operation_state(&other), OperationState::Waiting);

    gov.cancel(&t.admin(), &id);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
    let again = gov.propose(&t.admin(), &AdminOperation::SetPositionLimits(l), &s);
    assert_eq!(again, id);
    assert_eq!(gov.get_operation_state(&again), OperationState::Waiting);
}

/// G-1 recovery isolation: the recovery id is unreachable through `execute`
/// (target is governance, 34), a different canceller vector is Unset (4002),
/// the owner-canceller cannot cancel it even after the full 518_400-ledger
/// delay (sidecar still live, 46), and after execution the id is Unset and a
/// second `execute_canceller_reset` is refused (4002).
#[test]
fn recovery_operation_is_isolated_from_other_paths() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let fresh = Address::generate(&t.env);
    let set: Vec<Address> = soroban_sdk::vec![&t.env, fresh.clone()];
    let s = salt(&t.env, 7);
    let id = gov.propose_canceller_reset(&set, &s);
    assert_eq!(
        id,
        gov.hash_operation(
            &t.governance,
            &Symbol::new(&t.env, "reset_cancellers"),
            &soroban_sdk::vec![&t.env, set.clone().into_val(&t.env)],
            &salt(&t.env, 0),
            &s
        )
    );
    advance(&t, RECOVERY);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    assert_contract_error(
        exec_raw(
            &t,
            &t.governance,
            "reset_cancellers",
            soroban_sdk::vec![&t.env, set.clone().into_val(&t.env)],
            salt(&t.env, 0),
            s.clone(),
        ),
        errors::codes::INTERNAL_ERROR,
    );
    let other: Vec<Address> = soroban_sdk::vec![&t.env, Address::generate(&t.env)];
    assert_contract_error(
        flatten(gov.try_execute_canceller_reset(&None, &other, &s)),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    assert_contract_error(
        flatten(gov.try_cancel(&t.admin(), &id)),
        GenericError::OperationNotCancellable as u32,
    );

    gov.execute_canceller_reset(&None, &set, &s);
    let canceller = Symbol::new(&t.env, "CANCELLER");
    assert!(gov.has_role(&fresh, &canceller));
    assert!(gov.has_role(&t.admin(), &canceller));
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
    assert_contract_error(
        flatten(gov.try_execute_canceller_reset(&None, &set, &s)),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
}

/// G-2 re-resolution: two delay updates valid at propose time; after the
/// larger one executes, the smaller one is re-validated at execute and refused
/// (39), while a controller op scheduled under the old delay keeps its fixed
/// ready ledger.
#[test]
fn execute_self_revalidates_against_current_state() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let d = gov.get_min_delay();
    assert_eq!(d, 50);
    let big = AdminOperation::UpdateGovDelay(100);
    let small = AdminOperation::UpdateGovDelay(60);
    let s_big = salt(&t.env, 8);
    let s_small = salt(&t.env, 9);
    gov.propose(&t.admin(), &big, &s_big);
    let small_id = gov.propose(&t.admin(), &small, &s_small);
    let l = limits(4, 3);
    let s_lim = salt(&t.env, 10);
    let lim_id = gov.propose(
        &t.admin(),
        &AdminOperation::SetPositionLimits(l.clone()),
        &s_lim,
    );
    advance(&t, d);

    exec_self(&t, &big, s_big).expect("larger delay applies");
    assert_eq!(gov.get_min_delay(), 100);
    assert_contract_error(
        exec_self(&t, &small, s_small),
        GenericError::InvalidTimelockDelay as u32,
    );
    assert_eq!(
        gov.get_operation_state(&small_id),
        OperationState::Ready,
        "refusal does not consume it"
    );

    assert_eq!(gov.get_operation_state(&lim_id), OperationState::Ready);
    exec_raw(
        &t,
        &t.controller,
        "set_position_limits",
        limits_args(&t, &l),
        salt(&t.env, 0),
        s_lim,
    )
    .expect("op scheduled under the old delay keeps its ready ledger");
}
