//! Second opinion on Specula governance CR-8: with the configured minimum
//! delay at the `configs/networks.json` value (12 ledgers) and the compiled
//! Sensitive floor also 12, an owner-proposed `GrantGovRole(PROPOSER)`
//! (Sensitive tier) and `UpdateGovDelay` (Standard tier) are both executable
//! by anyone, unsigned, exactly 12 ledgers after `propose`, and not at 11.
//!
//! The second test pins the remedy's arithmetic: `UpdateGovDelay(34_560)` is
//! itself a Standard operation that waits only 12 ledgers, cannot lower the
//! minimum (INV-AUTH-05), and once executed every later Sensitive operation
//! waits `max(34_560, 12)`.
//!
//! The harness governance registers with a 50-ledger minimum, so each test
//! registers a second governance contract at the network value. Only self
//! operations are exercised, which need no controller behind the contract.

use governance_interface::{AdminOperation, OperationState, RoleArgs};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Symbol};
use test_harness::errors::GenericError;
use test_harness::{assert_contract_error, errors, LendingTest};

/// `configs/networks.json` `timelock_min_delay_ledgers`, both networks.
const NETWORK_MIN_DELAY: u32 = 12;
/// `governance::constants::TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS` at HEAD.
const SENSITIVE_FLOOR: u32 = 12;
/// `governance::constants::TIMELOCK_MIN_DELAY_LEDGERS`, the documented target.
const TARGET_MIN_DELAY: u32 = 34_560;
/// `governance::constants::TIMELOCK_MAX_DELAY_LEDGERS`.
const MAX_DELAY: u32 = 241_920;
const INVALID_TIMELOCK_DELAY: u32 = GenericError::InvalidTimelockDelay as u32;

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
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

/// A governance contract whose constructor minimum is the network value.
fn network_governance(t: &LendingTest) -> (Address, governance_interface::GovernanceClient<'_>) {
    let owner = Address::generate(&t.env);
    let id = t
        .env
        .register(governance::Governance, (owner.clone(), NETWORK_MIN_DELAY));
    let gov = governance_interface::GovernanceClient::new(&t.env, &id);
    assert_eq!(gov.get_min_delay(), NETWORK_MIN_DELAY);
    (owner, gov)
}

fn grant_proposer(t: &LendingTest, who: &Address) -> AdminOperation {
    AdminOperation::GrantGovRole(RoleArgs {
        account: who.clone(),
        role: Symbol::new(&t.env, "PROPOSER"),
    })
}

/// CR-8 as stated: the Sensitive tier resolves to `max(12, 12) = 12`, so a
/// role grant proposed by the owner is Waiting at +11 and, with every mocked
/// authorization removed, executes at +12 with no executor and no signature.
#[test]
fn grant_proposer_executes_unsigned_twelve_ledgers_after_proposal() {
    let t = LendingTest::new().build();
    let (owner, gov) = network_governance(&t);
    let mallory = Address::generate(&t.env);
    let op = grant_proposer(&t, &mallory);
    let s = salt(&t, 1);
    let proposer_role = Symbol::new(&t.env, "PROPOSER");

    let id = gov.propose(&owner, &op, &s);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);

    advance(&t, SENSITIVE_FLOOR - 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    assert_contract_error(
        flatten(gov.try_execute_self(&None, &op, &s)),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    assert!(!gov.has_role(&mallory, &proposer_role));

    advance(&t, 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    // No mocked authorization from here on: any `require_auth` would fail.
    t.env.set_auths(&[]);
    assert!(
        gov.try_propose(&owner, &op, &salt(&t, 9)).is_err(),
        "auth mocking is really cleared: the owner can no longer propose"
    );
    gov.execute_self(&None, &op, &s);

    assert!(gov.has_role(&mallory, &proposer_role));
    assert_eq!(gov.get_operation_state(&id), OperationState::Unset);
}

/// The remedy's arithmetic. `UpdateGovDelay` is Standard, so at the network
/// value it also waits only 12 ledgers and is executable unsigned; it cannot
/// lower the minimum or exceed the cap; after it, a Sensitive grant waits the
/// full 34_560 ledgers.
#[test]
fn update_delay_is_standard_tier_and_lifts_later_sensitive_waits() {
    let t = LendingTest::new().build();
    let (owner, gov) = network_governance(&t);

    assert_contract_error(
        flatten(gov.try_propose(
            &owner,
            &AdminOperation::UpdateGovDelay(NETWORK_MIN_DELAY - 1),
            &salt(&t, 1),
        )),
        INVALID_TIMELOCK_DELAY,
    );
    assert_contract_error(
        flatten(gov.try_propose(
            &owner,
            &AdminOperation::UpdateGovDelay(MAX_DELAY + 1),
            &salt(&t, 2),
        )),
        INVALID_TIMELOCK_DELAY,
    );

    let raise = AdminOperation::UpdateGovDelay(TARGET_MIN_DELAY);
    let s = salt(&t, 3);
    let id = gov.propose(&owner, &raise, &s);

    advance(&t, NETWORK_MIN_DELAY - 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Waiting);
    advance(&t, 1);
    assert_eq!(gov.get_operation_state(&id), OperationState::Ready);

    t.env.set_auths(&[]);
    gov.execute_self(&None, &raise, &s);
    assert_eq!(gov.get_min_delay(), TARGET_MIN_DELAY);

    t.env.mock_all_auths();
    let mallory = Address::generate(&t.env);
    let grant = grant_proposer(&t, &mallory);
    let s2 = salt(&t, 4);
    let id2 = gov.propose(&owner, &grant, &s2);

    advance(&t, SENSITIVE_FLOOR);
    assert_eq!(
        gov.get_operation_state(&id2),
        OperationState::Waiting,
        "the 12-ledger window is closed once the minimum is 34_560"
    );
    assert_contract_error(
        flatten(gov.try_execute_self(&None, &grant, &s2)),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );

    advance(&t, TARGET_MIN_DELAY - SENSITIVE_FLOOR - 1);
    assert_eq!(gov.get_operation_state(&id2), OperationState::Waiting);
    advance(&t, 1);
    assert_eq!(gov.get_operation_state(&id2), OperationState::Ready);
    gov.execute_self(&None, &grant, &s2);
    assert!(gov.has_role(&mallory, &Symbol::new(&t.env, "PROPOSER")));
}
