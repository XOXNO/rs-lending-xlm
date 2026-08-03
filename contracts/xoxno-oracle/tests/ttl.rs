#![cfg(test)]
extern crate std;

mod common;
use common::*;

use ::common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env};
use xoxno_oracle::{XoxnoOracle, XoxnoOracleClient};

#[test]
fn entrypoint_renews_oracle_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let signer = Address::generate(&env);
    let contract_id = env.register(
        XoxnoOracle,
        (admin, vec![&env, signer], 1u32, TEST_RESOLUTION),
    );
    let client = XoxnoOracleClient::new(&env, &contract_id);

    let initial = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert!(
        initial < TTL_THRESHOLD_INSTANCE,
        "precondition: fresh instance TTL ({initial}) must sit below the renewal threshold"
    );

    client.set_resolution(&TEST_RESOLUTION);

    let renewed = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(
        renewed, TTL_BUMP_INSTANCE,
        "entrypoint must re-arm the instance bump"
    );
}

#[test]
fn submit_price_arms_shared_ttl_on_submission_key() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);

    let key = MirrorKey::LatestSubmission(feed.clone(), signers[0].clone());
    let ttl = env.as_contract(&client.address, || env.storage().persistent().get_ttl(&key));
    assert!(
        ttl >= TTL_THRESHOLD_SHARED,
        "submission key TTL ({ttl}) must be lifted above the shared threshold"
    );
    assert_eq!(
        ttl, TTL_BUMP_SHARED,
        "write path must arm the shared bump on the submission key"
    );
}

#[test]
fn submit_price_renews_known_feed_allowlist_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    let decay: u32 = TTL_BUMP_SHARED - 20_000;
    advance_ledger_sequence(&env, decay);

    let index_key = MirrorKey::FeedIndex(feed.clone());
    let slot_key = MirrorKey::FeedAt(0);
    let (index_before, slot_before) = env.as_contract(&client.address, || {
        (
            env.storage().persistent().get_ttl(&index_key),
            env.storage().persistent().get_ttl(&slot_key),
        )
    });
    assert!(
        index_before < TTL_THRESHOLD_SHARED && slot_before < TTL_THRESHOLD_SHARED,
        "precondition: aged allowlist TTLs ({index_before}, {slot_before}) must sit below the renewal threshold"
    );

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);

    let (index_after, slot_after) = env.as_contract(&client.address, || {
        (
            env.storage().persistent().get_ttl(&index_key),
            env.storage().persistent().get_ttl(&slot_key),
        )
    });
    assert_eq!(
        index_after, TTL_BUMP_SHARED,
        "submit must re-arm the FeedIndex allowlist gate"
    );
    assert_eq!(
        slot_after, TTL_BUMP_SHARED,
        "submit must re-arm the paired FeedAt slot"
    );
}
