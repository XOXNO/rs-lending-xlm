//! TTL renewal behavior: entrypoints re-arm instance storage, and the write
//! path arms the shared bump on hot persistent keys.
//!
//! `Env::default()` grants fresh entries a TTL (4095 ledgers) far below
//! `TTL_THRESHOLD_*` (5 days = 86_400 ledgers), so a renewing call must lift
//! the TTL to exactly `TTL_BUMP_*` and a skipped renewal is directly visible.

#![cfg(test)]
extern crate std;

mod common;
use common::*;

use ::common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
use soroban_sdk::Env;

#[test]
fn entrypoint_renews_oracle_instance_ttl() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let initial = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert!(
        initial < TTL_THRESHOLD_INSTANCE,
        "precondition: fresh instance TTL ({initial}) must sit below the renewal threshold"
    );

    // Any entrypoint renews the instance; `set_resolution` is the simplest.
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
