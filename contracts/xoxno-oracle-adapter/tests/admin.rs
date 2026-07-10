//! Constructor / admin / signer-set behavior.

#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle_adapter::{Error, XoxnoOracle, XoxnoOracleClient};

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Env, IntoVal};

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_threshold_of_zero() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signers = soroban_sdk::vec![&env, Address::generate(&env)];
    env.register(XoxnoOracle, (admin, signers, 0u32, TEST_RESOLUTION));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_threshold_above_signer_count() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let signers = soroban_sdk::vec![&env, Address::generate(&env)];
    env.register(XoxnoOracle, (admin, signers, 2u32, TEST_RESOLUTION));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_duplicate_signers() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let dup = Address::generate(&env);
    // Same address twice -> has_duplicate -> InvalidThreshold.
    let signers = soroban_sdk::vec![&env, dup.clone(), dup];
    env.register(XoxnoOracle, (admin, signers, 1u32, TEST_RESOLUTION));
}

#[test]
fn renounce_ownership_clears_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    assert!(client.get_owner().is_some());
    client.renounce_ownership();
    assert!(client.get_owner().is_none());
}

#[test]
fn remove_signer_rejected_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);

    let result = client.try_remove_signer(&signers[0]);
    assert_eq!(result, Err(Ok(Error::CannotRemoveBelowThreshold)));
}

#[test]
fn remove_signer_succeeds_above_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);

    client.remove_signer(&signers[0]);
    // Removing a second one would now drop below threshold.
    let result = client.try_remove_signer(&signers[1]);
    assert_eq!(result, Err(Ok(Error::CannotRemoveBelowThreshold)));
}

#[test]
fn only_owner_can_initiate_ownership_transfer() {
    let env = Env::default();
    // The constructor itself does not call `require_auth`, so registering
    // the contract needs no mocked auths at all.
    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let signers = soroban_sdk::vec![&env, signer.clone()];
    let contract_id = env.register(XoxnoOracle, (admin.clone(), signers, 1u32, TEST_RESOLUTION));
    let client = XoxnoOracleClient::new(&env, &contract_id);

    // Mock auth as `non_owner` invoking `transfer_ownership` — OZ's
    // `enforce_owner_auth` calls `require_auth` on the STORED owner, which
    // does not match the authorized address `non_owner`, so the host must
    // reject this invocation.
    let non_owner = Address::generate(&env);
    let new_owner = Address::generate(&env);
    let live_until_ledger = 1000u32;
    env.mock_auths(&[MockAuth {
        address: &non_owner,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "transfer_ownership",
            args: (new_owner.clone(), live_until_ledger).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = client.try_transfer_ownership(&new_owner, &live_until_ledger);
    assert!(result.is_err());

    // The real owner succeeds in initiating the transfer, but ownership does
    // not move until `new_owner` calls `accept_ownership` (2-step handshake).
    env.mock_auths(&[MockAuth {
        address: &admin,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "transfer_ownership",
            args: (new_owner.clone(), live_until_ledger).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    client.transfer_ownership(&new_owner, &live_until_ledger);
    assert_eq!(client.get_owner(), Some(admin));

    env.mock_auths(&[MockAuth {
        address: &new_owner,
        invoke: &MockAuthInvoke {
            contract: &contract_id,
            fn_name: "accept_ownership",
            args: soroban_sdk::vec![&env],
            sub_invokes: &[],
        },
    }]);
    client.accept_ownership();
    assert_eq!(client.get_owner(), Some(new_owner));
}

#[test]
fn set_max_submission_age_enforces_floor_and_ttl_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    // Below the 60s floor is rejected.
    assert_eq!(
        client.try_set_max_submission_age_seconds(&59u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );
    // Above the cache TTL (default 86_400s) is rejected.
    assert_eq!(
        client.try_set_max_submission_age_seconds(&86_401u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );
    // The floor and the ceiling themselves are accepted.
    client.set_max_submission_age_seconds(&60u64);
    client.set_max_submission_age_seconds(&86_400u64);
}

#[test]
fn set_max_stale_cannot_drop_below_submission_age() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    // Default submission-age window is 900s; the cache TTL cannot go tighter.
    assert_eq!(
        client.try_set_max_stale_seconds(&899u64),
        Err(Ok(Error::InvalidSubmissionAge))
    );
    // Equal to the window is accepted.
    client.set_max_stale_seconds(&900u64);
}

#[test]
fn only_admin_can_call_add_feed() {
    let env = Env::default();
    let (client, admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    let not_admin = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &not_admin,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "add_feed",
            args: (feed_id(&env), asset.clone()).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let result = client.try_add_feed(&feed_id(&env), &asset);
    assert!(result.is_err());

    // The real admin succeeds.
    env.mock_all_auths();
    let _ = admin;
    client.add_feed(&feed_id(&env), &asset);
    assert_eq!(client.assets().len(), 1);
}
