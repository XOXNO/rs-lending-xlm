//! `submit_price` / `submit_prices` / aggregation behavior.

#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle_adapter::Error;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, String};

#[test]
fn submit_price_rejects_non_registered_signer() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 2, 1);

    let outsider = Address::generate(&env);
    let result = client.try_submit_price(&outsider, &feed_id(&env), &100i128, &1_000u64);
    assert_eq!(result, Err(Ok(Error::NotAuthorizedSigner)));
}

#[test]
fn submit_price_rejects_non_positive_price() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let result = client.try_submit_price(&signers[0], &feed_id(&env), &0i128, &1_000u64);
    assert_eq!(result, Err(Ok(Error::InvalidPrice)));

    let result = client.try_submit_price(&signers[0], &feed_id(&env), &(-5i128), &1_000u64);
    assert_eq!(result, Err(Ok(Error::InvalidPrice)));
}

#[test]
fn submit_price_rejects_package_timestamp_beyond_future_skew() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    advance_ledger_seconds(&env, 100_000);

    let now = env.ledger().timestamp();
    // 61s ahead exceeds the 60s skew window; ms conversion.
    let too_future_ms = (now + 61) * 1_000;
    let result = client.try_submit_price(&signers[0], &feed_id(&env), &100i128, &too_future_ms);
    assert_eq!(result, Err(Ok(Error::FutureTimestamp)));

    // Within the skew window is accepted.
    let ok_ms = (now + 60) * 1_000;
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &ok_ms);
}

#[test]
fn aggregate_not_produced_until_threshold_reached() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    // Before any submissions.
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    // One submission — below threshold of 2.
    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    // Second submission reaches threshold.
    client.submit_price(&signers[1], &feed, &102i128, &1_000u64);
    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(101u128));
}

#[test]
fn median_odd_count() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 3);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &300i128, &2_000u64);
    client.submit_price(&signers[2], &feed, &200i128, &3_000u64);

    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(200u128));
    // Oldest contributing package_timestamp: the aggregate is only as fresh as
    // its stalest included submission.
    assert_eq!(data.package_timestamp, 1_000u64);
}

#[test]
fn median_even_count() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 4, 4);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &300i128, &1_000u64);
    client.submit_price(&signers[2], &feed, &200i128, &1_000u64);
    client.submit_price(&signers[3], &feed, &400i128, &1_000u64);

    // sorted: 100, 200, 300, 400 -> middle two are 200, 300 -> avg 250
    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(250u128));
}

#[test]
fn stale_submission_excluded_from_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    // Both submit at ledger time 0 (package_timestamp in ms).
    client.submit_price(&signers[0], &feed, &100i128, &0u64);
    client.submit_price(&signers[1], &feed, &200i128, &0u64);
    // Threshold met — aggregate exists.
    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(150u128));

    // Advance ledger time well past MaxStaleSeconds (default 86400s).
    advance_ledger_seconds(&env, 90_000);

    // A fresh submission from signer[0] triggers recompute; signer[1]'s
    // now-stale submission (package_timestamp 0) must be excluded, dropping
    // the kept count below threshold (2), so CurrentAggregate is cleared.
    client.submit_price(&signers[0], &feed, &500i128, &90_000_000u64);

    // Fail-safe: dropping below threshold removes the cached aggregate, so the
    // read returns NoDataForFeed rather than serving the old poisoned value.
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );
}

#[test]
fn bulk_read_fails_entirely_if_any_feed_missing() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    // feed_b never submitted.

    let result = client.try_read_price_data(&vec![&env, feed_a, feed_b]);
    assert_eq!(expect_error(result), Error::NoDataForFeed);
}

#[test]
fn read_price_history_newest_first_and_capped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    // Push 15 aggregates (cap is 12); prices 1..=15 at increasing timestamps.
    for i in 1..=15u64 {
        advance_ledger_seconds(&env, 10);
        let ts_ms = env.ledger().timestamp() * 1000;
        client.submit_price(&signers[0], &feed, &(i as i128), &ts_ms);
    }

    let history = client.read_price_history(&feed, &100u32);
    // Capped at 12 entries.
    assert_eq!(history.len(), 12);
    // Newest first: last pushed price was 15, oldest retained is 4 (15-12+1).
    assert_eq!(history.get(0).unwrap().price.to_u128(), Some(15u128));
    assert_eq!(history.get(11).unwrap().price.to_u128(), Some(4u128));
}

#[test]
fn read_price_history_errors_when_absent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let result = client.try_read_price_history(&feed_id(&env), &10u32);
    assert_eq!(result, Err(Ok(Error::NoDataForFeed)));
}

// MAX_SUBMITTED_PRICE is 1e24 (crate-private); tests use the literal directly.
const MAX_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

#[test]
fn submit_price_rejects_price_above_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let result = client.try_submit_price(&signers[0], &feed_id(&env), &(MAX_PRICE + 1), &1_000u64);
    assert_eq!(result, Err(Ok(Error::PriceOutOfRange)));

    // Exactly at the ceiling is accepted.
    client.submit_price(&signers[0], &feed_id(&env), &MAX_PRICE, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed_id(&env)).price.to_u128(),
        Some(MAX_PRICE as u128)
    );
}

#[test]
fn submit_prices_rejects_price_above_ceiling_upfront() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    let feeds = vec![&env, feed_a.clone(), feed_b];
    let prices = vec![&env, 100i128, MAX_PRICE + 1];

    let result = client.try_submit_prices(&signers[0], &feeds, &prices, &1_000u64);
    assert_eq!(expect_error(result), Error::PriceOutOfRange);
    // Checked upfront: the valid first price is not stored on failure.
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed_a)),
        Error::NoDataForFeed
    );
}

#[test]
fn median_even_count_large_prices_no_overflow() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    // Two near-ceiling prices whose unchecked `a + b` would abort under
    // overflow-checks-adjacent large arithmetic; the overflow-safe midpoint
    // `a + (b - a)/2` returns the exact median without panicking.
    let a = MAX_PRICE - 2;
    let b = MAX_PRICE;
    client.submit_price(&signers[0], &feed, &b, &1_000u64);
    client.submit_price(&signers[1], &feed, &a, &1_000u64);

    // median = a + (b - a)/2 = MAX_PRICE - 1
    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some((MAX_PRICE - 1) as u128));
}

#[test]
fn remove_signer_refreshes_aggregate_excluding_removed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    client.submit_price(&signers[2], &feed, &300i128, &1_000u64);
    // median of [100, 200, 300] = 200
    assert_eq!(client.read_price_data_for_feed(&feed).price.to_u128(), Some(200u128));

    // Removing the high outlier recomputes immediately over [100, 200] (still
    // meets threshold 2) -> median 150, without waiting for MaxStaleSeconds.
    client.remove_signer(&signers[2]);
    assert_eq!(client.read_price_data_for_feed(&feed).price.to_u128(), Some(150u128));
}

#[test]
fn remove_signer_only_recomputes_touched_feeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");

    // feed_a: signers 0,1,2 ; feed_b: signers 1,2 only (signer 0 never touches it).
    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed_a, &200i128, &1_000u64);
    client.submit_price(&signers[2], &feed_a, &300i128, &1_000u64);
    client.submit_price(&signers[1], &feed_b, &10i128, &1_000u64);
    client.submit_price(&signers[2], &feed_b, &20i128, &1_000u64);

    assert_eq!(client.read_price_data_for_feed(&feed_a).price.to_u128(), Some(200u128));
    assert_eq!(client.read_price_data_for_feed(&feed_b).price.to_u128(), Some(15u128));

    // Only feed_a is in signer 0's SignerFeeds, so only feed_a recomputes
    // (median of [200, 300] = 250); feed_b is left exactly as-is.
    client.remove_signer(&signers[0]);
    assert_eq!(client.read_price_data_for_feed(&feed_a).price.to_u128(), Some(250u128));
    assert_eq!(client.read_price_data_for_feed(&feed_b).price.to_u128(), Some(15u128));
}

#[test]
fn remove_signer_clears_aggregate_when_dropping_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    // Exactly two of the three signers submit — meets threshold 2.
    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert_eq!(client.read_price_data_for_feed(&feed).price.to_u128(), Some(150u128));

    // Removing signer[1] keeps the signer count (3 -> 2) at threshold, but
    // leaves only signer[0]'s fresh submission (1 < 2). Fail-safe: the cached
    // aggregate is cleared rather than left serving signer[1]'s poisoned price.
    client.remove_signer(&signers[1]);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );
}

#[test]
fn raising_threshold_invalidates_below_quorum_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 1);
    let feed = feed_id(&env);

    // 1-of-N aggregate readable under threshold 1.
    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    assert_eq!(client.read_price_data_for_feed(&feed).price.to_u128(), Some(100u128));

    // Raising the threshold to 2 re-validates every known feed; this feed now
    // has only one fresh submission (1 < 2), so its aggregate is cleared.
    client.set_threshold(&2u32);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    // A second fresh submission restores quorum and the aggregate reappears.
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert_eq!(client.read_price_data_for_feed(&feed).price.to_u128(), Some(150u128));
}

#[test]
fn losing_quorum_clears_twap_history() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    let asset = xlm_asset(&env);
    client.add_feed(&feed, &asset);

    // Quorum met: SEP-40 TWAP history is populated.
    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert!(client.prices(&asset, &12).is_some());

    // Dropping below quorum clears History too, so the TWAP path can't keep
    // driving prices off samples that no longer meet the current quorum.
    client.remove_signer(&signers[1]);
    assert!(client.prices(&asset, &12).is_none());
}
