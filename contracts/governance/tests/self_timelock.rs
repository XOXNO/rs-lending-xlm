use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, Error, Symbol};
use stellar_governance::timelock::OperationState;

use crate::access::EXECUTOR_ROLE;
use crate::constants::{TIMELOCK_MIN_DELAY_LEDGERS, TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS};
use crate::op::{AdminOperation, RoleArgs};
use crate::test_support::{register, upload_controller_wasm, zero_salt, ZERO_SALT};

#[test]
fn propose_update_delay_schedules_waiting_operation() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _gov_id, gov) = register(&env, delay);

    let salt = zero_salt(&env);
    let id = gov.propose(&admin, &AdminOperation::UpdateGovDelay(15u32), &salt);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn propose_update_delay_rejects_shortening_below_the_floor() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);

    let salt = zero_salt(&env);
    gov.propose(&admin, &AdminOperation::UpdateGovDelay(5u32), &salt);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn propose_update_delay_rejects_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);

    let salt = zero_salt(&env);
    gov.propose(&admin, &AdminOperation::UpdateGovDelay(0u32), &salt);
}

#[test]
#[should_panic(expected = "Error(Contract, #39)")]
fn propose_update_delay_rejects_above_max_cap() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);

    let salt = zero_salt(&env);
    let over_max = crate::constants::TIMELOCK_MAX_DELAY_LEDGERS + 1;
    gov.propose(&admin, &AdminOperation::UpdateGovDelay(over_max), &salt);
}

#[test]
fn propose_governance_upgrade_uses_sensitive_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);
    let salt = zero_salt(&env);
    let hash = BytesN::from_array(&env, &[9u8; 32]);

    let current = env.ledger().sequence();
    let id = gov.propose(&admin, &AdminOperation::UpgradeGov(hash), &salt);

    assert_eq!(
        gov.get_operation_ledger(&id),
        current + TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS
    );
}

#[test]
fn execute_governance_upgrade_replaces_contract_code() {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env.cost_estimate().disable_resource_limits();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);
    let salt = zero_salt(&env);
    let hash = upload_controller_wasm(&env);
    let op = AdminOperation::UpgradeGov(hash);

    gov.propose(&admin, &op, &salt);
    env.ledger()
        .with_mut(|l| l.sequence_number += TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS);
    gov.execute_self(&Some(admin), &op, &salt);

    let upgraded = controller::ControllerClient::new(&env, &gov.address);
    assert_eq!(upgraded.get_app_version(), 1);
}

#[test]
fn execute_update_delay_applies_after_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _gov_id, gov) = register(&env, delay);

    let salt = zero_salt(&env);
    let id = gov.propose(&admin, &AdminOperation::UpdateGovDelay(15u32), &salt);
    env.ledger().with_mut(|l| l.sequence_number += delay);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    gov.execute_self(
        &Some(admin.clone()),
        &AdminOperation::UpdateGovDelay(15u32),
        &salt,
    );
    assert_eq!(gov.get_min_delay(), 15u32);
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

#[test]
fn update_delay_shortens_to_the_floor_through_the_timelock_but_not_below() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = TIMELOCK_MIN_DELAY_LEDGERS * 2;
    let (admin, _gov_id, gov) = register(&env, delay);

    let below = AdminOperation::UpdateGovDelay(TIMELOCK_MIN_DELAY_LEDGERS - 1);
    assert_eq!(
        gov.try_propose(&admin, &below, &zero_salt(&env)),
        Err(Ok(Error::from_contract_error(39)))
    );

    let salt = zero_salt(&env);
    let floor = AdminOperation::UpdateGovDelay(TIMELOCK_MIN_DELAY_LEDGERS);
    let id = gov.propose(&admin, &floor, &salt);
    env.ledger().with_mut(|l| l.sequence_number += delay - 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    env.ledger().with_mut(|l| l.sequence_number += 1);
    gov.execute_self(&Some(admin.clone()), &floor, &salt);
    assert_eq!(gov.get_min_delay(), TIMELOCK_MIN_DELAY_LEDGERS);
}

#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn propose_grant_governance_role_rejects_unknown_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);
    let salt = zero_salt(&env);
    gov.propose(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: admin.clone(),
            role: Symbol::new(&env, "KEEPER"),
        }),
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #2000)")]
fn non_proposer_cannot_propose_governance_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, _gov_id, gov) = register(&env, 10);
    let stranger = Address::generate(&env);
    let salt = zero_salt(&env);
    gov.propose(
        &stranger,
        &AdminOperation::UpgradeGov(BytesN::from_array(&env, &ZERO_SALT)),
        &salt,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn propose_governance_upgrade_rejects_zero_hash() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, _gov_id, gov) = register(&env, 10);
    let salt = zero_salt(&env);
    gov.propose(
        &admin,
        &AdminOperation::UpgradeGov(BytesN::from_array(&env, &[0u8; 32])),
        &salt,
    );
}

#[test]
fn execute_grant_governance_role_after_delay() {
    let env = Env::default();
    env.mock_all_auths();
    let delay = 10u32;
    let (admin, _gov_id, gov) = register(&env, delay);
    let grantee = Address::generate(&env);
    let role = Symbol::new(&env, EXECUTOR_ROLE);
    let salt = zero_salt(&env);

    gov.propose(
        &admin,
        &AdminOperation::GrantGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
        &salt,
    );
    env.ledger()
        .with_mut(|l| l.sequence_number += delay.max(TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS));
    gov.execute_self(
        &Some(admin.clone()),
        &AdminOperation::GrantGovRole(RoleArgs {
            account: grantee.clone(),
            role: role.clone(),
        }),
        &salt,
    );
    assert!(gov.has_role(&grantee, &role));
}
