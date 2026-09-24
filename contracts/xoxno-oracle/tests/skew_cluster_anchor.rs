//! The skew-cluster anchor is clamped to ledger time, so a future-dated
//! submission inside the future-skew bound cannot drag the cluster window
//! forward and evict the honest submissions.
#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle::Error;

/// Five signers, threshold two: two colluders at the future-skew bound do not
/// evict the three honest submissions, and the aggregate stays at the honest price.
#[test]
fn future_dated_submission_cannot_evict_the_honest_cohort() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 10_000);
    let (client, _admin, signers) = setup(&env, 5, 2);
    let feed = feed_id(&env);

    let now_ms = env.ledger().timestamp() * 1_000;
    for signer in signers.iter() {
        client.submit_price(signer, &feed, &100i128, &now_ms);
    }

    // A lull, then two colluders submit a divergent price at the future bound.
    advance_ledger_seconds(&env, 880);
    let future_ms = (env.ledger().timestamp() + 60) * 1_000;
    client.submit_price(&signers[3], &feed, &500i128, &future_ms);
    client.submit_price(&signers[4], &feed, &500i128, &future_ms);

    // The three honest submissions (880 s old) stay in the 900 s skew window.
    // The five-entry cluster is below 2 * (5 - 2) + 1 = 7 entries and fails the
    // spread bound, so the honest aggregate of 100 is retained.
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128),
        "the honest majority must still determine the median"
    );
}

/// Three signers, threshold two, minimum 61 s skew: a third signer at the future
/// bound does not evict the honest pair submitted 2 s earlier, so its submission
/// writes a fresh aggregate over all three.
#[test]
fn one_future_dated_signer_cannot_evict_the_honest_pair() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 10_000);
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    client.set_max_relative_skew_seconds(&61u64);

    let honest_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &honest_ms);
    client.submit_price(&signers[1], &feed, &100i128, &honest_ms);

    // Measured from the future timestamp, the honest pair is 62 s old.
    advance_ledger_seconds(&env, 2);
    let now_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[2], &feed, &100i128, &(now_ms + 60_000));

    let aggregate = client.read_price_data_for_feed(&feed);
    assert_eq!(
        aggregate.write_timestamp, now_ms,
        "the future-dated submission must aggregate with the honest pair"
    );
    assert_eq!(aggregate.package_timestamp, honest_ms);
}

/// Control: the skew filter still evicts old submissions that are not future-dated.
/// Threshold is the full signer set: without eviction the three submissions form
/// an aggregate, so `NoDataForFeed` proves the eviction.
#[test]
fn stale_submission_outside_the_skew_window_is_still_evicted() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 10_000);
    let (client, _admin, signers) = setup(&env, 3, 3);
    let feed = feed_id(&env);

    // Wide age window, narrow skew: 200s old is within age but outside skew.
    client.set_max_submission_age_seconds(&86_400u64);
    client.set_max_relative_skew_seconds(&61u64);

    // Two stale signers publish 500 at now-200s.
    let stale_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &500i128, &stale_ms);
    client.submit_price(&signers[1], &feed, &500i128, &stale_ms);

    advance_ledger_seconds(&env, 200);

    // One fresh signer publishes 100 at now. The anchor clamps to now; the two
    // stale 500s are >61s older and evicted, leaving one submission < threshold.
    let fresh_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[2], &feed, &100i128, &fresh_ms);

    assert_eq!(
        client.try_read_price_data_for_feed(&feed),
        Err(Ok(Error::NoDataForFeed)),
        "stale submissions outside the skew window must be evicted, keeping the \
         cluster below threshold"
    );
}
