//! F-2 regression: the skew-cluster anchor is clamped to ledger time, so a
//! future-dated (but within-future-skew) submission cannot drag the cluster
//! window forward and evict the honest cohort.
//!
//! These assert the DEFENDED behaviour. On the pre-clamp tree they fail; the
//! full pre-fix reproduction (asserting the vulnerable behaviour) is preserved
//! at `docs/audits/artifacts/oracle_skew_cluster_anchor_reproduction.rs`.
#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle::Error;

/// Core defended property: with a five-signer / threshold-two feed, two
/// colluders publishing a divergent price at the future-skew bound must not
/// evict the three honest, still-valid submissions. The honest majority keeps
/// the median.
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

    // The three honest submissions (880s < 900s age limit) stay in the cluster.
    // Median of {100, 100, 100, 500, 500} = 100.
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128),
        "the honest majority must still determine the median"
    );
}

/// One future-dated signer must not clear a two-signer honest quorum.
#[test]
fn one_future_dated_signer_cannot_clear_the_feed() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 10_000);
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    let now_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &now_ms);
    client.submit_price(&signers[1], &feed, &100i128, &now_ms);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128),
        "quorum of 2 honest signers prices the feed"
    );

    advance_ledger_seconds(&env, 1);
    let future_ms = (env.ledger().timestamp() + 60) * 1_000;
    client.submit_price(&signers[2], &feed, &100i128, &future_ms);

    // The honest pair, one second old, stays clustered; the feed is still live.
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128),
        "a future-dated third submission must not evict the honest pair"
    );
}

/// Non-vacuity control: the skew filter must still evict a genuinely stale
/// (old, not future-dated) submission, else the tests above prove nothing. Two
/// stale signers outside the skew window are dropped, leaving one fresh signer
/// below threshold and clearing the feed.
#[test]
fn stale_submission_outside_the_skew_window_is_still_evicted() {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 10_000);
    let (client, _admin, signers) = setup(&env, 3, 2);
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
        "stale submissions outside the skew window must be evicted, dropping the \
         cluster below threshold"
    );
}
