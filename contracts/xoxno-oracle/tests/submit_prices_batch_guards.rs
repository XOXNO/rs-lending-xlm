#![cfg(test)]
//! `submit_prices` is the path the keepers use. Its guards are a second copy
//! of the `submit_price` chain, in a different order, and only the price and
//! length faults were pinned. These pin the rest, each with its own error,
//! and that a rejected batch stores nothing.
extern crate std;

mod common;
use common::*;

use xoxno_oracle::Error;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, String, Vec};

const MAX_AGE_SECONDS: u64 = 900;

struct Batch {
    feeds: Vec<String>,
    prices: Vec<i128>,
    feed_a: String,
    feed_b: String,
}

fn two_feed_batch(client: &xoxno_oracle::XoxnoOracleClient<'static>, env: &Env) -> Batch {
    register_extra_feeds(client, env, &["A/USD", "B/USD"]);
    let feed_a = String::from_str(env, "A/USD");
    let feed_b = String::from_str(env, "B/USD");
    Batch {
        feeds: vec![env, feed_a.clone(), feed_b.clone()],
        prices: vec![env, 100i128, 200i128],
        feed_a,
        feed_b,
    }
}

fn assert_nothing_priced(client: &xoxno_oracle::XoxnoOracleClient<'static>, b: &Batch) {
    for feed in [&b.feed_a, &b.feed_b] {
        assert_eq!(
            expect_error(client.try_read_price_data_for_feed(feed)),
            Error::NoDataForFeed,
            "a rejected batch must store nothing"
        );
    }
}

#[test]
fn submit_prices_rejects_package_timestamp_beyond_future_skew() {
    let env = Env::default();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let b = two_feed_batch(&client, &env);
    advance_ledger_seconds(&env, 100_000);
    let now = env.ledger().timestamp();

    let result = client.try_submit_prices(&signers[0], &b.feeds, &b.prices, &((now + 61) * 1_000));
    assert_eq!(expect_error(result), Error::FutureTimestamp);
    assert_nothing_priced(&client, &b);

    // The last millisecond of second +60 is still inside the skew.
    client.submit_prices(
        &signers[0],
        &b.feeds,
        &b.prices,
        &((now + 60) * 1_000 + 999),
    );
}

#[test]
fn submit_prices_rejects_stale_package_timestamp() {
    let env = Env::default();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let b = two_feed_batch(&client, &env);
    advance_ledger_seconds(&env, 100_000);
    let now = env.ledger().timestamp();

    let stale_ms = (now - MAX_AGE_SECONDS - 1) * 1_000;
    let result = client.try_submit_prices(&signers[0], &b.feeds, &b.prices, &stale_ms);
    assert_eq!(expect_error(result), Error::StaleSubmission);
    assert_nothing_priced(&client, &b);

    client.submit_prices(
        &signers[0],
        &b.feeds,
        &b.prices,
        &((now - MAX_AGE_SECONDS) * 1_000),
    );
}

#[test]
fn submit_prices_rejects_unregistered_signer() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 2, 1);
    let b = two_feed_batch(&client, &env);

    let outsider = Address::generate(&env);
    let result = client.try_submit_prices(&outsider, &b.feeds, &b.prices, &1_000u64);
    assert_eq!(expect_error(result), Error::NotAuthorizedSigner);
    assert_nothing_priced(&client, &b);
}

#[test]
fn submit_prices_rejects_unknown_feed_and_stores_no_earlier_entry() {
    let env = Env::default();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let b = two_feed_batch(&client, &env);

    // Known feed first, unknown feed last: the good entry must not land.
    let feeds = vec![&env, b.feed_a.clone(), String::from_str(&env, "NOPE/USD")];
    let result = client.try_submit_prices(&signers[0], &feeds, &b.prices, &1_000u64);
    assert_eq!(expect_error(result), Error::FeedNotKnown);
    assert_nothing_priced(&client, &b);
}

#[test]
fn submit_prices_rejects_a_timestamp_regression_on_any_entry() {
    let env = Env::default();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let b = two_feed_batch(&client, &env);
    advance_ledger_seconds(&env, 100_000);
    let now_ms = env.ledger().timestamp() * 1_000;

    // Only B has a newer stored package; A has none.
    client.submit_price(&signers[0], &b.feed_b, &200i128, &now_ms);

    let regressed_ms = now_ms - 1;
    let result = client.try_submit_prices(&signers[0], &b.feeds, &b.prices, &regressed_ms);
    assert_eq!(expect_error(result), Error::StaleSubmission);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&b.feed_a)),
        Error::NoDataForFeed,
        "entry A must not land when entry B regresses"
    );
    let kept = client.read_price_data_for_feed(&b.feed_b);
    assert_eq!(kept.price.to_u128(), Some(200u128));
    assert_eq!(kept.package_timestamp, now_ms);

    // An equal timestamp is not a regression.
    client.submit_prices(&signers[0], &b.feeds, &b.prices, &now_ms);
}

/// Batch twin of `lone_late_submission_cannot_take_the_feed_offline`: the
/// keepers submit through `submit_prices`, so the AQUA incident guard
/// (`QuorumMiss::Retain`) must hold on this path for every feed in the batch.
#[test]
fn lone_late_batch_submission_cannot_take_the_feeds_offline() {
    let env = Env::default();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let b = two_feed_batch(&client, &env);

    advance_ledger_seconds(&env, 10_000);
    let quorum_ms = env.ledger().timestamp() * 1_000;
    let quorum_prices = vec![&env, 34_585i128, 70_000i128];
    client.submit_prices(&signers[1], &b.feeds, &quorum_prices, &quorum_ms);
    client.submit_prices(&signers[2], &b.feeds, &quorum_prices, &quorum_ms);

    // 22 minutes on: both quorum submissions are past the 900 s window.
    advance_ledger_seconds(&env, 1_320);
    let lone_ms = env.ledger().timestamp() * 1_000;
    client.submit_prices(
        &signers[0],
        &b.feeds,
        &vec![&env, 34_287i128, 69_000i128],
        &lone_ms,
    );

    for (feed, price) in [(&b.feed_a, 34_585u128), (&b.feed_b, 70_000u128)] {
        let after = client.read_price_data_for_feed(feed);
        assert_eq!(
            after.price.to_u128(),
            Some(price),
            "a lone late batch must neither price the feed nor unprice it"
        );
        assert_eq!(after.package_timestamp, quorum_ms);
    }
}
