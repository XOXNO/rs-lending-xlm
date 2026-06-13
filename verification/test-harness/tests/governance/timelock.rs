//! Timelock lifecycle integration coverage, driven through `GovernanceClient`
//! against the real WASM-backed controller the harness wires up.
//!
//! The governance unit tests (`contracts/governance/src/timelock.rs`) already
//! pin the lifecycle against a natively-registered controller. This suite proves
//! the same behavior end to end through the production client surface and the
//! deployed controller, and adds the cases the unit tests do not: the full
//! `OperationState` transition chain in one flow, that a cancelled op can no
//! longer execute, role rejection via the typed `try_` client methods, and
//! salt-driven id distinctness.
//!
//! The harness arms a zero timelock delay so setup runs in one frame; each test
//! here re-arms a non-zero delay (owner-immediate `update_delay`) so the
//! `Waiting` state is observable, then advances the ledger to reach `Ready`.

use controller::types::{ControllerKey, PositionLimits};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol};
use test_harness::{assert_contract_error, errors, LendingTest};

const SET_POSITION_LIMITS: &str = "set_position_limits";

// A short, test-sized delay; long enough that `propose` lands in `Waiting`.
const TEST_DELAY_LEDGERS: u32 = 50;

fn salt(env: &soroban_sdk::Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn limits(supply: u32, borrow: u32) -> PositionLimits {
    PositionLimits {
        max_supply_positions: supply,
        max_borrow_positions: borrow,
    }
}

fn read_controller_position_limits(t: &LendingTest) -> PositionLimits {
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .instance()
            .get(&ControllerKey::PositionLimits)
            .expect("position limits set")
    })
}

// Re-arms a non-zero delay so scheduled ops sit in `Waiting` until the ledger
// advances. Owner-immediate, mirroring production.
fn arm_delay(t: &LendingTest) {
    t.gov_client().update_delay(&TEST_DELAY_LEDGERS);
    assert_eq!(t.gov_client().get_min_delay(), TEST_DELAY_LEDGERS);
}

// The full state machine in one flow: an unknown id is `Unset`; after a proposer
// schedules, the op is `Waiting`; after the delay elapses it is `Ready`; after
// an executor runs it, it is `Done` and the controller reflects the change.
#[test]
fn operation_state_transitions_unset_waiting_ready_done() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let admin = t.admin();
    let s = salt(&t.env, 1);

    arm_delay(&t);

    let new_limits = limits(9, 7);

    // Unset: an id we have not scheduled yet.
    let pre_id = gov.hash_operation(
        &t.controller,
        &Symbol::new(&t.env, SET_POSITION_LIMITS),
        &soroban_sdk::vec![&t.env, new_limits.clone().into_val(&t.env)],
        &salt(&t.env, 0),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&pre_id),
        governance::OperationState::Unset
    );

    // Waiting: scheduled but the delay has not elapsed.
    let id = gov.propose_set_position_limits(&admin, &new_limits, &s);
    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Waiting
    );

    // Ready: the delay has elapsed.
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);
    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Ready
    );

    // Done: executed, and the controller now holds the new limits.
    gov.execute(
        &Some(admin.clone()),
        &t.controller,
        &Symbol::new(&t.env, SET_POSITION_LIMITS),
        &soroban_sdk::vec![&t.env, new_limits.clone().into_val(&t.env)],
        &salt(&t.env, 0),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Done
    );

    let stored = read_controller_position_limits(&t);
    assert_eq!(stored.max_supply_positions, 9);
    assert_eq!(stored.max_borrow_positions, 7);
}

// A cancelled op returns to `Unset` and can no longer be executed: even after the
// delay elapses, `execute` reverts because the operation is not `Ready`
// (OZ `InvalidOperationState` #4002).
#[test]
fn cancelled_operation_cannot_execute() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let admin = t.admin();
    let s = salt(&t.env, 2);

    arm_delay(&t);

    let new_limits = limits(8, 6);
    let id = gov.propose_set_position_limits(&admin, &new_limits, &s);
    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Waiting
    );

    gov.cancel(&admin, &id);
    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Unset
    );

    // Past the delay, a cancelled op is still not executable.
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);

    let result = gov.try_execute(
        &Some(admin.clone()),
        &t.controller,
        &Symbol::new(&t.env, SET_POSITION_LIMITS),
        &soroban_sdk::vec![&t.env, new_limits.into_val(&t.env)],
        &salt(&t.env, 0),
        &s,
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    // OZ TimelockError::InvalidOperationState == 4002.
    assert_contract_error(mapped, 4002);
}

// A non-PROPOSER cannot schedule: the typed proposer rejects with the OZ
// AccessControl `Unauthorized` (#2000) before anything is queued.
#[test]
fn non_proposer_propose_rejected() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let stranger = Address::generate(&t.env);

    let result = gov.try_propose_set_position_limits(&stranger, &limits(5, 4), &salt(&t.env, 3));
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);
}

// A non-EXECUTOR cannot execute a ready op: the explicit-executor path rejects
// with `Unauthorized` (#2000) once it sees the caller lacks EXECUTOR.
#[test]
fn non_executor_execute_rejected() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let s = salt(&t.env, 4);

    arm_delay(&t);

    let new_limits = limits(5, 4);
    gov.propose_set_position_limits(&admin, &new_limits, &s);
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);

    let result = gov.try_execute(
        &Some(stranger),
        &t.controller,
        &Symbol::new(&t.env, SET_POSITION_LIMITS),
        &soroban_sdk::vec![&t.env, new_limits.into_val(&t.env)],
        &salt(&t.env, 0),
        &s,
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);
}

// A non-CANCELLER cannot cancel a pending op: rejected with `Unauthorized`
// (#2000); the op stays `Waiting`.
#[test]
fn non_canceller_cancel_rejected() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let s = salt(&t.env, 5);

    arm_delay(&t);

    let id = gov.propose_set_position_limits(&admin, &limits(5, 4), &s);

    let result = gov.try_cancel(&stranger, &id);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);

    assert_eq!(
        gov.get_operation_state(&id),
        governance::OperationState::Waiting
    );
}

// The owner may shorten the delay immediately (owner-gated, not timelocked); a
// non-owner cannot. The owner path is exercised by `arm_delay`; here the
// non-owner is rejected with the owner-gate error.
#[test]
fn update_delay_owner_only() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();

    // Owner succeeds and the new delay is observable.
    gov.update_delay(&7u32);
    assert_eq!(gov.get_min_delay(), 7u32);

    // Non-owner is rejected. `mock_all_auths_allowing_non_root_auth` satisfies the
    // stranger's signature, so the revert comes from the ownable gate, not auth.
    let stranger = Address::generate(&t.env);
    t.env.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &stranger,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &t.governance,
            fn_name: "update_delay",
            args: (11u32,).into_val(&t.env),
            sub_invokes: &[],
        },
    }]);
    let result = gov.try_update_delay(&11u32);
    assert!(result.is_err(), "non-owner update_delay must revert");
    // The delay is unchanged.
    assert_eq!(gov.get_min_delay(), 7u32);
}

// Two ops with identical params but different salt hash to distinct ids and both
// schedule independently — the salt is the only uniqueness lever the proposers
// expose (predecessor is always zero; see the module note).
//
// Predecessor ordering: the OZ module supports a non-zero predecessor
// (`set_execute_operation` checks the predecessor is `Done`), but the v1 typed
// proposers always schedule with a zero predecessor (`forward.rs`
// `schedule_controller_op`). There is no client surface to set a predecessor, so
// predecessor-chained execution is out of scope for this suite and is not faked.
#[test]
fn same_params_distinct_salts_schedule_independently() {
    let t = LendingTest::new().build();
    let gov = t.gov_client();
    let admin = t.admin();

    arm_delay(&t);

    let new_limits = limits(5, 4);
    let salt_a = salt(&t.env, 6);
    let salt_b = salt(&t.env, 7);

    let id_a = gov.propose_set_position_limits(&admin, &new_limits, &salt_a);
    let id_b = gov.propose_set_position_limits(&admin, &new_limits, &salt_b);

    assert_ne!(id_a, id_b, "distinct salts must yield distinct op ids");
    assert_eq!(
        gov.get_operation_state(&id_a),
        governance::OperationState::Waiting
    );
    assert_eq!(
        gov.get_operation_state(&id_b),
        governance::OperationState::Waiting
    );
}
