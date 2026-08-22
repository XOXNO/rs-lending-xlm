use controller::types::{ControllerKey, PositionLimits};
use governance_interface::{
    AdminOperation, ConfigureAssetOracleArgs, EditToleranceArgs, TransferOwnershipArgs,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, IntoVal, Symbol};
use test_harness::{
    assert_contract_error, errors, map_try_ok_unit, reflector_primary_anchor_config, usd,
    usdc_preset, LendingTest, DEFAULT_TOLERANCE,
};

const SET_POSITION_LIMITS: &str = "set_position_limits";

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

#[test]
fn operation_state_transitions_unset_waiting_ready_unset() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let s = salt(&t.env, 1);

    assert_harness_delay(&t);

    let new_limits = limits(4, 3);

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

    let id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS);
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Ready
    );

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
    assert_eq!(stored.max_supply_positions, 4);
    assert_eq!(stored.max_borrow_positions, 3);
}

#[test]
fn cancelled_operation_cannot_execute() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let s = salt(&t.env, 2);

    assert_harness_delay(&t);

    let new_limits = limits(4, 2);
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

    assert_contract_error(mapped, errors::TIMELOCK_UNEXPECTED_STATE);
}

/// The property a timelock exists for: a Waiting operation must refuse to
/// execute, both immediately and one ledger short of the delay, and the very
/// same call must succeed once the delay elapses.
#[test]
fn operation_cannot_execute_before_the_delay_elapses() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let s = salt(&t.env, 9);

    assert_harness_delay(&t);

    let new_limits = limits(4, 3);
    let id = gov.propose(
        &admin,
        &AdminOperation::SetPositionLimits(new_limits.clone()),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );

    let try_execute = |t: &LendingTest| {
        let result = t.gov_iface_client().try_execute(
            &Some(admin.clone()),
            &t.controller,
            &Symbol::new(&t.env, SET_POSITION_LIMITS),
            &soroban_sdk::vec![&t.env, new_limits.clone().into_val(&t.env)],
            &salt(&t.env, 0),
            &s,
        );
        match result {
            Ok(res) => res.map_err(|e| e.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    };

    assert_contract_error(try_execute(&t), errors::TIMELOCK_UNEXPECTED_STATE);

    t.env
        .ledger()
        .with_mut(|l| l.sequence_number += TEST_DELAY_LEDGERS - 1);
    assert_contract_error(try_execute(&t), errors::TIMELOCK_UNEXPECTED_STATE);

    t.env.ledger().with_mut(|l| l.sequence_number += 1);
    gov.execute(
        &Some(admin.clone()),
        &t.controller,
        &Symbol::new(&t.env, SET_POSITION_LIMITS),
        &soroban_sdk::vec![&t.env, new_limits.clone().into_val(&t.env)],
        &salt(&t.env, 0),
        &s,
    );
    let stored = read_controller_position_limits(&t);
    assert_eq!(stored.max_supply_positions, 4);
    assert_eq!(stored.max_borrow_positions, 3);
}

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

#[test]
fn non_canceller_cancel_rejected() {
    let t = LendingTest::new().build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let stranger = Address::generate(&t.env);
    let s = salt(&t.env, 5);

    assert_harness_delay(&t);

    let id = gov.propose(&admin, &AdminOperation::SetPositionLimits(limits(5, 4)), &s);

    let mapped = map_try_ok_unit(gov.try_cancel(&stranger, &id));
    assert_contract_error(mapped, errors::UNAUTHORIZED);

    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Waiting
    );
}

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

const SET_ASSET_ORACLE: &str = "set_oracle";

#[test]
fn resolve_market_oracle_view_matches_scheduled_and_executes() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let gov = t.gov_iface_client();
    let admin = t.admin();
    let asset = t.resolve_asset("USDC");
    let s = salt(&t.env, 8);

    assert_harness_delay(&t);

    let cfg = reflector_primary_anchor_config(
        &t.env,
        &t.mock_reflector,
        &asset,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );

    let resolved =
        gov.resolve_asset_oracle(&controller::types::PriceKey::Token(asset.clone()), &cfg);

    let id = gov.propose(
        &admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: controller::types::PriceKey::Token(asset.clone()),
            oracle: cfg,
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

    t.mock_reflector_client().set_price(&asset, &usd(1));

    gov.execute(
        &Some(admin.clone()),
        &t.price_aggregator,
        &Symbol::new(&t.env, SET_ASSET_ORACLE),
        &soroban_sdk::vec![
            &t.env,
            controller::types::PriceKey::Token(asset.clone()).into_val(&t.env),
            resolved.clone().into_val(&t.env),
        ],
        &salt(&t.env, 0),
        &s,
    );
    assert_eq!(
        gov.get_operation_state(&id),
        governance_interface::OperationState::Unset
    );

    let stored = t.market_oracle_config(&asset);
    assert_eq!(stored, resolved);
}

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
            key: controller::types::PriceKey::Token(asset.clone()),
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

    t.mock_reflector_client().set_price(&asset, &usd(1));

    gov.execute(
        &Some(admin.clone()),
        &t.price_aggregator,
        &Symbol::new(&t.env, "set_tolerance"),
        &soroban_sdk::vec![
            &t.env,
            controller::types::PriceKey::Token(asset.clone()).into_val(&t.env),
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
