#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle::Error;

use ::common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address, Env, String, Symbol};

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

    let too_future_ms = (now + 61) * 1_000;
    let result = client.try_submit_price(&signers[0], &feed_id(&env), &100i128, &too_future_ms);
    assert_eq!(result, Err(Ok(Error::FutureTimestamp)));

    let ok_ms = (now + 60) * 1_000;
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &ok_ms);
}

#[test]
fn aggregate_not_produced_until_threshold_reached() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    client.submit_price(&signers[1], &feed, &102i128, &1_000u64);
    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(100u128));
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

    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(200u128));
}

#[test]
fn submission_at_exact_inclusion_window_boundary_is_aggregated() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    advance_ledger_seconds(&env, 2_000);

    let now = env.ledger().timestamp();
    let boundary_ms = (now - 900) * 1_000;
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &boundary_ms);
    assert_eq!(
        client
            .read_price_data_for_feed(&feed_id(&env))
            .price
            .to_u128(),
        Some(100u128)
    );
}

#[test]
fn relative_skew_boundary_is_inclusive() {
    let env = Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 2_000);
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    client.set_max_relative_skew_seconds(&100u64);
    let newest_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &newest_ms);
    client.submit_price(&signers[1], &feed, &200i128, &(newest_ms - 100 * 1_000));

    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

#[test]
fn median_even_count_with_odd_gap_rounds_toward_lower_middle() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &101i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

#[test]
fn stale_submission_excluded_from_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    advance_ledger_seconds(&env, 1_000);
    let initial_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &initial_ms);
    client.submit_price(&signers[1], &feed, &200i128, &initial_ms);

    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(100u128));

    advance_ledger_seconds(&env, 901);

    client.set_max_relative_skew_seconds(&900u64);
    let fresh_ms = (env.ledger().timestamp() - 1) * 1_000;
    client.submit_price(&signers[0], &feed, &500i128, &fresh_ms);

    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );
}

#[test]
fn lagging_signer_does_not_pin_feed_freshness() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    advance_ledger_seconds(&env, 1_000);

    let t0_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[2], &feed, &200i128, &t0_ms);

    advance_ledger_seconds(&env, 1_000);
    let t1_ms = env.ledger().timestamp() * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &t1_ms);
    client.submit_price(&signers[1], &feed, &102i128, &t1_ms);

    let data = client.read_price_data_for_feed(&feed);

    assert_eq!(data.price.to_u128(), Some(100u128));

    assert_eq!(data.package_timestamp, t1_ms);
}

#[test]
fn submit_price_rejects_stale_package_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    advance_ledger_seconds(&env, 2_000);

    let now = env.ledger().timestamp();

    let too_old_ms = (now - 901) * 1_000;
    let result = client.try_submit_price(&signers[0], &feed_id(&env), &100i128, &too_old_ms);
    assert_eq!(result, Err(Ok(Error::StaleSubmission)));

    let ok_ms = (now - 900) * 1_000;
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &ok_ms);
}

#[test]
fn bulk_read_fails_entirely_if_any_feed_missing() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);
    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);

    let result = client.try_read_price_data(&vec![&env, feed_a, feed_b]);
    assert_eq!(expect_error(result), Error::NoDataForFeed);
}

#[test]
fn read_price_history_newest_first_and_capped() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    for i in 1..=15u64 {
        advance_ledger_seconds(&env, TEST_RESOLUTION as u64);
        let ts_ms = env.ledger().timestamp() * 1000;
        client.submit_price(&signers[0], &feed, &(i as i128), &ts_ms);
    }

    let history = client.read_price_history(&feed, &100u32);

    assert_eq!(history.len(), 12);

    assert_eq!(history.get(0).unwrap().price.to_u128(), Some(15u128));
    assert_eq!(history.get(11).unwrap().price.to_u128(), Some(4u128));
}

#[test]
fn sub_resolution_submissions_overwrite_same_history_bucket() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    for (offset, price) in [(0u64, 100i128), (30, 101), (60, 102)] {
        advance_ledger_seconds(&env, offset);
        let ts_ms = env.ledger().timestamp() * 1000;
        client.submit_price(&signers[0], &feed, &price, &ts_ms);
    }
    let history = client.read_price_history(&feed, &100u32);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().price.to_u128(), Some(102u128));

    advance_ledger_seconds(&env, TEST_RESOLUTION as u64);
    let ts_ms = env.ledger().timestamp() * 1000;
    client.submit_price(&signers[0], &feed, &200i128, &ts_ms);
    let history = client.read_price_history(&feed, &100u32);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().price.to_u128(), Some(200u128));
    assert_eq!(history.get(1).unwrap().price.to_u128(), Some(102u128));
}

#[test]
fn read_price_history_errors_when_absent() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let result = client.try_read_price_history(&feed_id(&env), &10u32);
    assert_eq!(result, Err(Ok(Error::NoDataForFeed)));
}

const MAX_PRICE: i128 = 1_000_000_000_000_000_000_000_000;

#[test]
fn submit_price_rejects_price_above_ceiling() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let result = client.try_submit_price(&signers[0], &feed_id(&env), &(MAX_PRICE + 1), &1_000u64);
    assert_eq!(result, Err(Ok(Error::PriceOutOfRange)));

    client.submit_price(&signers[0], &feed_id(&env), &MAX_PRICE, &1_000u64);
    assert_eq!(
        client
            .read_price_data_for_feed(&feed_id(&env))
            .price
            .to_u128(),
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
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);
    let feeds = vec![&env, feed_a.clone(), feed_b];
    let prices = vec![&env, 100i128, MAX_PRICE + 1];

    let result = client.try_submit_prices(&signers[0], &feeds, &prices, &1_000u64);
    assert_eq!(expect_error(result), Error::PriceOutOfRange);

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

    let a = MAX_PRICE - 2;
    let b = MAX_PRICE;
    client.submit_price(&signers[0], &feed, &b, &1_000u64);
    client.submit_price(&signers[1], &feed, &a, &1_000u64);

    let data = client.read_price_data_for_feed(&feed);
    assert_eq!(data.price.to_u128(), Some(a as u128));
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

    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(200u128)
    );

    client.remove_signer(&signers[2]);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

#[test]
fn remove_signer_only_recomputes_touched_feeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);

    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed_a, &200i128, &1_000u64);
    client.submit_price(&signers[2], &feed_a, &300i128, &1_000u64);
    client.submit_price(&signers[1], &feed_b, &10i128, &1_000u64);
    client.submit_price(&signers[2], &feed_b, &20i128, &1_000u64);

    assert_eq!(
        client.read_price_data_for_feed(&feed_a).price.to_u128(),
        Some(200u128)
    );
    assert_eq!(
        client.read_price_data_for_feed(&feed_b).price.to_u128(),
        Some(10u128)
    );

    client.remove_signer(&signers[0]);
    assert_eq!(
        client.read_price_data_for_feed(&feed_a).price.to_u128(),
        Some(200u128)
    );
    assert_eq!(
        client.read_price_data_for_feed(&feed_b).price.to_u128(),
        Some(10u128)
    );
}

#[test]
fn remove_signer_clears_aggregate_when_dropping_below_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

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

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    // The setter stores the threshold and nothing else: sweeping every feed in
    // the same transaction grows the footprint with the feed count and would
    // eventually make the setter permanently uncallable. So the aggregate formed
    // under the old threshold is still served here -- the window the batched
    // sweep trades for a bounded footprint.
    client.set_threshold(&2u32);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    // Once the sweep runs, the single submission is below the new quorum and the
    // feed must stop serving a price rather than serve one the threshold no
    // longer justifies.
    client.recompute_feeds(&vec![&env, feed.clone()]);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

#[test]
fn losing_quorum_clears_twap_history() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    let asset = xlm_asset(&env);
    client.add_feed(&feed, &asset);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert!(client.prices(&asset, &12).is_some());

    client.remove_signer(&signers[1]);
    assert!(client.prices(&asset, &12).is_none());
}

#[test]
fn submit_prices_stores_multiple_feeds() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);
    let feeds = vec![&env, feed_a.clone(), feed_b.clone()];
    let prices = vec![&env, 100i128, 200i128];

    client.submit_prices(&signers[0], &feeds, &prices, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed_a).price.to_u128(),
        Some(100u128)
    );
    assert_eq!(
        client.read_price_data_for_feed(&feed_b).price.to_u128(),
        Some(200u128)
    );
}

#[test]
fn submit_prices_rejects_length_mismatch() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);

    let feeds = vec![
        &env,
        String::from_str(&env, "A/USD"),
        String::from_str(&env, "B/USD"),
    ];
    let prices = vec![&env, 100i128];
    let result = client.try_submit_prices(&signers[0], &feeds, &prices, &1_000u64);
    assert_eq!(expect_error(result), Error::LengthMismatch);
}

#[test]
fn submit_prices_rejects_non_positive_price_upfront() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);

    let feed_a = String::from_str(&env, "A/USD");
    let feeds = vec![&env, feed_a.clone(), String::from_str(&env, "B/USD")];
    let prices = vec![&env, 100i128, 0i128];
    let result = client.try_submit_prices(&signers[0], &feeds, &prices, &1_000u64);
    assert_eq!(expect_error(result), Error::InvalidPrice);

    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed_a)),
        Error::NoDataForFeed
    );
}

#[test]
fn read_price_data_bulk_succeeds_when_all_present() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);

    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);
    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    client.submit_price(&signers[0], &feed_b, &200i128, &1_000u64);

    let results = client.read_price_data(&vec![&env, feed_a, feed_b]);
    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().price.to_u128(), Some(100u128));
    assert_eq!(results.get(1).unwrap().price.to_u128(), Some(200u128));
}

#[test]
fn read_price_data_for_feed_reports_stale_data() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &0u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    advance_ledger_seconds(&env, 86_401);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::StaleData
    );
}

#[test]
fn read_price_data_for_feed_accepts_exact_ttl_boundary_and_converts_ms() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    advance_ledger_seconds(&env, 10_000);
    let now = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed, &100i128, &(now * 1_000));

    advance_ledger_seconds(&env, 86_400);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    advance_ledger_seconds(&env, 1);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::StaleData
    );
}

#[test]
fn purge_feed_clears_submission_state_and_allows_reuse() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &1_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );

    client.purge_feed(&feed);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    client.register_feed(&feed);
    client.submit_price(&signers[0], &feed, &100i128, &2_000u64);
    client.submit_price(&signers[1], &feed, &200i128, &2_000u64);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(100u128)
    );
}

#[test]
fn purge_feed_rejects_unknown_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    let result = client.try_purge_feed(&String::from_str(&env, "NEVER/USD"));
    assert_eq!(expect_error(result), Error::FeedNotKnown);
}

#[test]
fn purge_feed_keeps_other_feeds_and_rewrites_indexes() {
    let env = Env::default();
    env.mock_all_auths();

    let (client, _admin, signers) = setup(&env, 2, 1);
    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);

    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    client.submit_price(&signers[0], &feed_b, &200i128, &1_000u64);

    client.purge_feed(&feed_a);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed_a)),
        Error::NoDataForFeed
    );
    assert_eq!(
        client.read_price_data_for_feed(&feed_b).price.to_u128(),
        Some(200u128)
    );
}

#[test]
fn purge_feed_prunes_only_the_purged_feed_from_signer_indexes() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 1);
    let feed_a = String::from_str(&env, "A/USD");
    let feed_b = String::from_str(&env, "B/USD");
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);

    client.submit_price(&signers[0], &feed_a, &100i128, &1_000u64);
    client.submit_price(&signers[0], &feed_b, &100i128, &1_000u64);
    client.submit_price(&signers[1], &feed_b, &300i128, &1_000u64);

    client.purge_feed(&feed_a);
    let s0_feeds: soroban_sdk::Vec<String> = env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .get(&MirrorKey::SignerFeeds(signers[0].clone()))
            .unwrap()
    });
    assert_eq!(s0_feeds, vec![&env, feed_b.clone()]);

    client.remove_signer(&signers[0]);
    assert_eq!(
        client.read_price_data_for_feed(&feed_b).price.to_u128(),
        Some(300u128)
    );
}

#[test]
fn purge_feed_removes_feed_from_known_feed_index() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let feed = feed_id(&env);

    client.submit_price(&signers[0], &feed, &100i128, &1_000u64);
    client.purge_feed(&feed);

    assert_eq!(
        expect_error(client.try_purge_feed(&feed)),
        Error::FeedNotKnown
    );
}

#[test]
fn submit_price_rejects_unregistered_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let unknown = String::from_str(&env, "UNKNOWN/USD");
    assert_eq!(
        client.try_submit_price(&signers[0], &unknown, &100i128, &1_000u64),
        Err(Ok(Error::FeedNotKnown))
    );
}

#[test]
fn submit_price_rejects_regression_in_package_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    advance_ledger_seconds(&env, 10_000);
    let now = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(now * 1_000));

    let older = (now - 10) * 1_000;
    assert_eq!(
        client.try_submit_price(&signers[0], &feed_id(&env), &200i128, &older),
        Err(Ok(Error::StaleSubmission))
    );
}

#[test]
fn lagging_in_window_signer_excluded_by_relative_skew() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    client.set_max_relative_skew_seconds(&450u64);
    advance_ledger_seconds(&env, 10_000);
    let now = env.ledger().timestamp();

    let fresh_ms = now * 1_000;
    let lagging_ms = (now - 500) * 1_000;
    client.submit_price(&signers[0], &feed, &100i128, &fresh_ms);
    client.submit_price(&signers[1], &feed, &102i128, &fresh_ms);
    client.submit_price(&signers[2], &feed, &10_000i128, &lagging_ms);

    let data = client.read_price_data_for_feed(&feed);

    assert_eq!(data.price.to_u128(), Some(100u128));
    assert_eq!(data.package_timestamp, fresh_ms);
}

#[test]
fn prices_returns_none_when_spot_is_stale() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &0u64);
    assert!(client.prices(&asset, &12).is_some());

    advance_ledger_seconds(&env, 86_401);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed_id(&env))),
        Error::StaleData
    );
    assert!(client.prices(&asset, &12).is_none());
    assert!(client.price(&asset, &0u64).is_none());
}

#[test]
fn add_feed_rejects_duplicate_feed_id_for_second_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset_a = xlm_asset(&env);
    let asset_b = ReflectorAsset::Other(Symbol::new(&env, "BTC"));
    client.add_feed(&feed_id(&env), &asset_a);
    assert_eq!(
        client.try_add_feed(&feed_id(&env), &asset_b),
        Err(Ok(Error::FeedAlreadyMapped))
    );
}

#[test]
fn remove_feed_wipes_price_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &1_000u64);
    assert!(client.lastprice(&asset).is_some());

    client.remove_feed(&asset);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed_id(&env))),
        Error::NoDataForFeed
    );

    assert_eq!(
        client.try_submit_price(&signers[0], &feed_id(&env), &100i128, &1_000u64),
        Err(Ok(Error::FeedNotKnown))
    );
}
