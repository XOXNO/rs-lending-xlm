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
//! The harness constructor arms a short non-zero delay so scheduled ops sit in
//! `Waiting` until the ledger advances to `Ready`.

use controller::types::{AssetOracleConfig, ControllerKey, PositionLimits};
use governance_interface::{
    AdminOperation, ConfigureOracleArgs, EditToleranceArgs, TransferOwnershipArgs,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol};
use test_harness::{
    assert_contract_error, errors, hub_asset, reflector_single_spot_config, usd, usdc_preset,
    LendingTest, DEFAULT_TOLERANCE,
};

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

fn assert_harness_delay(t: &LendingTest) {
    assert_eq!(t.gov_iface_client().get_min_delay(), TEST_DELAY_LEDGERS);
}

// The full state machine in one flow: an unknown id is `Unset`; after a proposer
// schedules, the op is `Waiting`; after the delay elapses it is `Ready`; after
// an executor runs it, storage is cleared back to `Unset` and the controller
// reflects the change.
#[test]
fn operation_state_transitions_unset_waiting_ready_unset() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let s = salt(&t.env, 1);

    assert_harness_delay(&t);

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
        governance_interface::OperationState::Unset
    );

    // Waiting: scheduled but the delay has not elapsed.
    let id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    // Ready: the delay has elapsed.
    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Ready
    );

    // Unset after execute: ledger entry removed; controller holds the new limits.
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
        governance_interface::OperationState::Unset
    );

    let stored = read_controller_position_limits(&t);
    assert_eq!(stored.max_supply_positions, 9);
    assert_eq!(stored.max_borrow_positions, 7);
}

// Cancelled op returns to `Unset`; execute reverts even after delay (#4002).
#[test]
fn cancelled_operation_cannot_execute() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let s = salt(&t.env, 2);

    assert_harness_delay(&t);

    let new_limits = limits(8, 6);
    let id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    gov.cancel(&admin, &id);
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Unset
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
    let gov = t.gov_iface_client();
    let stranger = Address::generate(&t.env);

    let result = gov.try_propose(
        &stranger,
        &AdminOperation::SetPositionLimits(limits(5, 4)),
        &salt(&t.env, 3),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);
}

#[test]
fn propose_transfer_controller_ownership_rejects_non_contract_owner() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let new_owner = Address::generate(&t.env);
    let live_until = t.env.ledger().sequence() + 1_000;

    let result = gov.try_propose(
        &admin,
        &AdminOperation::TransferCtrlOwnership(TransferOwnershipArgs {
            new_owner,
            live_until_ledger: live_until,
        }),
        &salt(&t.env, 9),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::NOT_SMART_CONTRACT);
}

// A non-EXECUTOR cannot execute a ready op: the explicit-executor path rejects
// with `Unauthorized` (#2000) once it sees the caller lacks EXECUTOR.
#[test]
fn non_executor_execute_rejected() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let s = salt(&t.env, 4);

    assert_harness_delay(&t);

    let new_limits = limits(5, 4);
    gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &s,
    );
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
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let s = salt(&t.env, 5);

    assert_harness_delay(&t);

    let id = gov.propose(&admin, &AdminOperation::SetPositionLimits(limits(5, 4)), &s);

    let result = gov.try_cancel(&stranger, &id);
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);

    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );
}

// Delay updates are timelocked and PROPOSER-gated; a stranger cannot schedule one.
#[test]
fn propose_update_delay_requires_proposer() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let stranger = Address::generate(&t.env);

    let result = gov.try_propose(
        &stranger,
        &AdminOperation::UpdateGovDelay(60u32),
        &salt(&t.env, 10),
    );
    let mapped = match result {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(mapped, errors::UNAUTHORIZED);
    assert_eq!(gov.get_min_delay(), TEST_DELAY_LEDGERS);
}

// Two ops with identical params but different salt hash to distinct ids and both
// schedule independently — the salt is the only uniqueness lever the proposers
// expose (predecessor is always zero; see the module note).
//
// Predecessor ordering: the OZ module can check a non-zero predecessor, but
// typed proposers always schedule with predecessor `0`. Predecessor chaining is
// unsupported; executed ops clear storage (no `Done` marker), so chaining could
// not work even if a caller passed a non-zero predecessor on `execute`.
#[test]
fn same_params_distinct_salts_schedule_independently() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();

    assert_harness_delay(&t);

    let new_limits = limits(5, 4);
    let salt_a = salt(&t.env, 6);
    let salt_b = salt(&t.env, 7);

    let id_a = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &salt_a,
    );
    let id_b = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &salt_b,
    );

    assert_ne!(id_a, id_b, "distinct salts must yield distinct op ids");
    assert_eq!(
        gov.get_operation_state(&id_a),
        governance_interface::OperationState::Waiting
    );
    assert_eq!(
        gov.get_operation_state(&id_b),
        governance_interface::OperationState::Waiting
    );
}

const SET_MARKET_ORACLE_CONFIG: &str = "set_oracle_config";

// The CLI timelock linchpin (TL-5b): the `resolve_market_oracle_config` view runs
// the SAME validate+probe path as `propose_configure_market_oracle`, so its output
// is byte-identical to the resolved `AssetOracleConfig` the proposer scheduled.
// If the two diverged by even one field the operation-id hash would not match and
// `execute` would revert (OZ `InvalidOperationState`). This proves: (1) the view
// output equals what the controller persists, and (2) feeding the view output as
// execute args drives a successful execute end to end.
#[test]
fn resolve_market_oracle_view_matches_scheduled_and_executes() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let asset = t.resolve_asset("USDC");
    let s = salt(&t.env, 8);

    assert_harness_delay(&t);

    let cfg = reflector_single_spot_config(
        &t.mock_reflector,
        &asset,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );

    // Resolve independently through the read-only view (no schedule, no state
    // change): this is exactly what the CLI invokes under `--send=no`.
    let resolved: AssetOracleConfig = gov.resolve_market_oracle_config(&asset, &cfg);

    // Schedule the same op through the proposer; it stores the resolved struct.
    let id = gov.propose(
        &admin,
        &AdminOperation::ConfigureMarketOracle(ConfigureOracleArgs {
            hub_asset: hub_asset(asset.clone()),
            cfg,
        }),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);

    // Execute with the VIEW's output as args. This only succeeds if the view
    // output hashes to the same operation id the proposer scheduled — i.e. it is
    // byte-identical to the scheduled args.
    gov.execute(
        &Some(admin.clone()),
        &t.price_aggregator,
        &Symbol::new(&t.env, SET_MARKET_ORACLE_CONFIG),
        &soroban_sdk::vec![
            &t.env,
            asset.clone().into_val(&t.env),
            resolved.clone().into_val(&t.env),
        ],
        &salt(&t.env, 0),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Unset
    );

    // The controller now stores exactly the view's resolved config.
    let stored = t.market_oracle_config(&asset);
    assert_eq!(stored, resolved);
}

// The tolerance view mirrors the proposer's `validate_and_calculate_tolerances`
// path, so its `OracleTolerance` output replays a `set_tolerance`
// op verbatim at execute time.
#[test]
fn resolve_oracle_tolerance_view_matches_scheduled_and_executes() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let asset = t.resolve_asset("USDC");
    let s = salt(&t.env, 9);

    assert_harness_delay(&t);

    let tolerance = DEFAULT_TOLERANCE.tolerance_bps;

    let resolved = gov.resolve_oracle_tolerance(&tolerance);

    let id = gov.propose(
        &admin,
        &AdminOperation::EditOracleTolerance(EditToleranceArgs {
            asset: asset.clone(),
            tolerance,
        }),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);

    gov.execute(
        &Some(admin.clone()),
        &t.price_aggregator,
        &Symbol::new(&t.env, "set_tolerance"),
        &soroban_sdk::vec![
            &t.env,
            asset.clone().into_val(&t.env),
            resolved.clone().into_val(&t.env),
        ],
        &salt(&t.env, 0),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Unset
    );

    let stored = t.market_oracle_config(&asset).tolerance;
    assert_eq!(stored, resolved);
}
