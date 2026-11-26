#![cfg(test)]
extern crate std;

mod common;
use common::*;

use std::panic::{catch_unwind, AssertUnwindSafe};

use xoxno_oracle::{Error, XoxnoOracleClient};

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{vec, Address, Env, IntoVal, String};

const HONEST_PRICE: i128 = 1_000_000;

fn now_ms(env: &Env) -> u64 {
    env.ledger().timestamp() * 1_000
}

/// Returns the stored aggregate price of `feed`, or `None` if the feed has no
/// aggregate.
fn stored_price(client: &XoxnoOracleClient<'static>, feed: &String) -> Option<u128> {
    match client.try_read_price_data_for_feed(feed) {
        Ok(Ok(data)) => data.price.to_u128(),
        Err(Ok(Error::NoDataForFeed)) => None,
        other => panic!("unexpected read result: {other:?}"),
    }
}

#[test]
fn lone_late_low_submission_cannot_move_the_aggregate() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    let (honest_a, honest_b, malicious) = (&signers[1], &signers[2], &signers[0]);

    advance_ledger_seconds(&env, 10_000);
    let ts_a = now_ms(&env);
    client.submit_price(honest_a, &feed, &HONEST_PRICE, &ts_a);
    advance_ledger_seconds(&env, 3);
    client.submit_price(honest_b, &feed, &HONEST_PRICE, &now_ms(&env));
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(HONEST_PRICE as u128)
    );

    // now = ts_a + 901 s. A is one second past the window, and B is 898 s old.
    advance_ledger_seconds(&env, 898);
    client.submit_price(malicious, &feed, &1i128, &now_ms(&env));
    let after = client.read_price_data_for_feed(&feed);
    assert_eq!(after.price.to_u128(), Some(HONEST_PRICE as u128));
    assert_eq!(after.package_timestamp, ts_a);

    advance_ledger_seconds(&env, 20_000);
    client.submit_price(malicious, &feed, &1i128, &now_ms(&env));
    let later = client.read_price_data_for_feed(&feed);
    assert_eq!(later.price.to_u128(), Some(HONEST_PRICE as u128));
    assert_eq!(later.package_timestamp, ts_a);
}

#[test]
fn one_signer_cannot_raise_the_price_or_beat_two_fresh_honest_entries() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    let (honest_a, honest_b, malicious) = (&signers[1], &signers[2], &signers[0]);

    advance_ledger_seconds(&env, 10_000);
    client.submit_price(honest_a, &feed, &HONEST_PRICE, &now_ms(&env));
    advance_ledger_seconds(&env, 3);
    client.submit_price(honest_b, &feed, &HONEST_PRICE, &now_ms(&env));

    advance_ledger_seconds(&env, 10);
    client.submit_price(malicious, &feed, &1i128, &now_ms(&env));
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(HONEST_PRICE as u128)
    );

    advance_ledger_seconds(&env, 888);
    client.submit_price(malicious, &feed, &(HONEST_PRICE * 10), &now_ms(&env));
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(HONEST_PRICE as u128)
    );
}

#[test]
fn lone_late_low_batch_submission_cannot_move_the_aggregates() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    register_extra_feeds(&client, &env, &["A/USD", "B/USD"]);
    let feeds = vec![
        &env,
        String::from_str(&env, "A/USD"),
        String::from_str(&env, "B/USD"),
    ];
    let honest = vec![&env, HONEST_PRICE, HONEST_PRICE * 2];

    advance_ledger_seconds(&env, 10_000);
    let ts_a = now_ms(&env);
    client.submit_prices(&signers[1], &feeds, &honest, &ts_a);
    advance_ledger_seconds(&env, 3);
    client.submit_prices(&signers[2], &feeds, &honest, &now_ms(&env));

    advance_ledger_seconds(&env, 898);
    client.submit_prices(
        &signers[0],
        &feeds,
        &vec![&env, 1i128, 1i128],
        &now_ms(&env),
    );

    for (feed, price) in feeds.iter().zip(honest.iter()) {
        let after = client.read_price_data_for_feed(&feed);
        assert_eq!(after.price.to_u128(), Some(price as u128));
        assert_eq!(after.package_timestamp, ts_a);
    }
}

#[test]
fn two_entry_cluster_at_the_spread_bound_prices_the_feed() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);

    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &now_ms(&env));
    client.submit_price(&signers[1], &feed, &1_020_000i128, &now_ms(&env));

    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(HONEST_PRICE as u128)
    );
}

#[test]
fn two_entry_cluster_past_the_spread_bound_is_a_quorum_miss() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);

    let quorum_ms = now_ms(&env);
    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &quorum_ms);
    client.submit_price(&signers[1], &feed, &HONEST_PRICE, &quorum_ms);

    advance_ledger_seconds(&env, 100);
    client.submit_price(&signers[1], &feed, &980_392i128, &now_ms(&env));
    let retained = client.read_price_data_for_feed(&feed);
    assert_eq!(retained.price.to_u128(), Some(HONEST_PRICE as u128));
    assert_eq!(retained.package_timestamp, quorum_ms);
    assert_eq!(retained.write_timestamp, quorum_ms);

    client.recompute_feeds(&vec![&env, feed.clone()]);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );
}

#[test]
fn full_cluster_with_one_outlier_uses_the_plain_median() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    client.submit_price(&signers[0], &feed, &1i128, &ts);
    client.submit_price(&signers[1], &feed, &HONEST_PRICE, &ts);
    assert_eq!(
        expect_error(client.try_read_price_data_for_feed(&feed)),
        Error::NoDataForFeed
    );

    client.submit_price(&signers[2], &feed, &1_010_000i128, &ts);
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(HONEST_PRICE as u128)
    );
}

#[test]
fn three_of_four_cluster_with_an_outlier_is_a_quorum_miss() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 4, 3);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    // 2 * 3 - 1 = 5 is above the signer count: every cluster is spread-checked.
    client.submit_price(&signers[0], &feed, &1i128, &ts);
    client.submit_price(&signers[1], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[2], &feed, &1_010_000i128, &ts);
    assert_eq!(stored_price(&client, &feed), None);

    client.submit_price(&signers[3], &feed, &HONEST_PRICE, &ts);
    assert_eq!(stored_price(&client, &feed), None);
}

#[test]
fn three_of_four_cluster_inside_the_spread_takes_the_lower_median() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 4, 3);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[1], &feed, &1_020_000i128, &ts);
    client.submit_price(&signers[2], &feed, &1_010_000i128, &ts);
    assert_eq!(stored_price(&client, &feed), Some(1_010_000));
}

#[test]
fn two_of_five_cluster_with_two_low_entries_is_a_quorum_miss() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 5, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    // 2 * (5 - 2) + 1 = 7 is above the signer count: every cluster is
    // spread-checked, and the first two entries stay the stored aggregate.
    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[1], &feed, &HONEST_PRICE, &ts);
    assert_eq!(stored_price(&client, &feed), Some(HONEST_PRICE as u128));

    client.submit_price(&signers[2], &feed, &1i128, &ts);
    client.submit_price(&signers[3], &feed, &1i128, &ts);
    assert_eq!(stored_price(&client, &feed), Some(HONEST_PRICE as u128));

    client.recompute_feeds(&vec![&env, feed.clone()]);
    assert_eq!(stored_price(&client, &feed), None);
}

#[test]
fn three_of_five_full_cluster_ignores_two_outliers() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 5, 3);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    // Four entries are below max(2 * (5 - 3) + 1, 2 * 3 - 1) = 5.
    client.submit_price(&signers[0], &feed, &1i128, &ts);
    client.submit_price(&signers[1], &feed, &(HONEST_PRICE * 10), &ts);
    client.submit_price(&signers[2], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[3], &feed, &HONEST_PRICE, &ts);
    assert_eq!(stored_price(&client, &feed), None);

    // The full signer set reaches it: two signers cannot hold the lower median.
    client.submit_price(&signers[4], &feed, &HONEST_PRICE, &ts);
    assert_eq!(stored_price(&client, &feed), Some(HONEST_PRICE as u128));
}

#[test]
fn every_agreeing_quorum_prices_the_feed() {
    for signer_count in 1..=7u32 {
        for threshold in 1..=signer_count {
            let env = Env::default();
            env.mock_all_auths();
            let (client, _admin, signers) = setup(&env, signer_count, threshold);
            let feed = feed_id(&env);
            advance_ledger_seconds(&env, 10_000);
            let ts = now_ms(&env);

            for (submitted, signer) in (1u32..).zip(signers.iter()) {
                client.submit_price(signer, &feed, &HONEST_PRICE, &ts);
                let expected = (submitted >= threshold).then_some(HONEST_PRICE as u128);
                assert_eq!(
                    stored_price(&client, &feed),
                    expected,
                    "{signer_count} signers, threshold {threshold}, {submitted} submitted"
                );
            }
        }
    }
}

#[test]
fn raising_the_threshold_does_not_free_a_split_cluster() {
    // Five signers, one absent, two honest and two colluders at half the price.
    let round = [
        HONEST_PRICE,
        HONEST_PRICE,
        HONEST_PRICE / 2,
        HONEST_PRICE / 2,
    ];
    for threshold in [3u32, 4, 5] {
        let env = Env::default();
        env.mock_all_auths();
        let (client, _admin, signers) = setup(&env, 5, threshold);
        let feed = feed_id(&env);
        advance_ledger_seconds(&env, 10_000);
        let ts = now_ms(&env);

        for (signer, price) in signers.iter().zip(round) {
            client.submit_price(signer, &feed, &price, &ts);
        }
        assert_eq!(stored_price(&client, &feed), None, "threshold {threshold}");
    }
}

#[test]
fn two_of_two_cluster_past_the_spread_is_a_quorum_miss() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 2, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[1], &feed, &1i128, &ts);
    assert_eq!(stored_price(&client, &feed), None);
}

#[test]
fn remove_signer_clears_an_aggregate_outside_the_spread_of_the_smaller_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &ts);
    client.submit_price(&signers[1], &feed, &(HONEST_PRICE * 2), &ts);
    assert_eq!(stored_price(&client, &feed), None);
    client.submit_price(&signers[2], &feed, &(HONEST_PRICE * 3), &ts);
    assert_eq!(stored_price(&client, &feed), Some(HONEST_PRICE as u128 * 2));

    // Two signers with threshold 2: the two entries left are below 2 * 2 - 1 = 3.
    client.remove_signer(&signers[2]);
    assert_eq!(stored_price(&client, &feed), None);
}

/// Submits the colluder price for `colluders` signers and the honest price for
/// `honest` signers at one package timestamp. `colluders_first` sets the order.
/// Panics if any stored aggregate, after any submission, is the colluder price.
fn assert_colluders_never_set_the_price(
    signer_count: u32,
    threshold: u32,
    colluders: u32,
    honest: u32,
    colluder_price: i128,
    colluders_first: bool,
) {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, signer_count, threshold);
    let feed = feed_id(&env);
    advance_ledger_seconds(&env, 10_000);
    let ts = now_ms(&env);

    let colluder_round = core::iter::repeat_n(colluder_price, colluders as usize);
    let honest_round = core::iter::repeat_n(HONEST_PRICE, honest as usize);
    let round: std::vec::Vec<i128> = if colluders_first {
        colluder_round.chain(honest_round).collect()
    } else {
        honest_round.chain(colluder_round).collect()
    };

    for (signer, price) in signers.iter().zip(round) {
        client.submit_price(signer, &feed, &price, &ts);
        let stored = stored_price(&client, &feed);
        assert!(
            stored.is_none() || stored == Some(HONEST_PRICE as u128),
            "{signer_count} signers, threshold {threshold}: {colluders} colluders with \
             {honest} honest stored {stored:?}"
        );
    }
}

#[test]
fn fewer_colluders_than_the_threshold_never_set_the_price() {
    for signer_count in 1..=7u32 {
        for threshold in 1..=signer_count {
            for colluders in 1..threshold {
                for honest in 1..=signer_count - colluders {
                    for colluder_price in [1i128, HONEST_PRICE * 10] {
                        for colluders_first in [true, false] {
                            assert_colluders_never_set_the_price(
                                signer_count,
                                threshold,
                                colluders,
                                honest,
                                colluder_price,
                                colluders_first,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn colluders_within_the_absent_bound_never_outvote_a_clustered_honest_entry() {
    // `signers - threshold` colluders can reach the threshold alone. They do
    // not set the price while an honest entry is in the cluster first.
    for signer_count in 2..=7u32 {
        for threshold in 1..signer_count {
            let absent_bound = signer_count - threshold;
            for colluders in threshold..=absent_bound {
                for honest in 1..=signer_count - colluders {
                    for colluder_price in [1i128, HONEST_PRICE * 10] {
                        assert_colluders_never_set_the_price(
                            signer_count,
                            threshold,
                            colluders,
                            honest,
                            colluder_price,
                            false,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn set_max_cluster_spread_bps_validates_the_range_and_drives_aggregation() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 3, 2);
    let feed = feed_id(&env);

    assert_eq!(client.max_cluster_spread_bps(), 200);
    assert_eq!(
        client.try_set_max_cluster_spread_bps(&0u32),
        Err(Ok(Error::InvalidClusterSpread))
    );
    assert_eq!(
        client.try_set_max_cluster_spread_bps(&10_001u32),
        Err(Ok(Error::InvalidClusterSpread))
    );
    client.set_max_cluster_spread_bps(&1u32);
    assert_eq!(client.max_cluster_spread_bps(), 1);
    client.set_max_cluster_spread_bps(&10_000u32);
    assert_eq!(client.max_cluster_spread_bps(), 10_000);

    client.set_max_cluster_spread_bps(&500u32);
    advance_ledger_seconds(&env, 10_000);
    client.submit_price(&signers[0], &feed, &HONEST_PRICE, &now_ms(&env));
    client.submit_price(&signers[1], &feed, &960_000i128, &now_ms(&env));
    assert_eq!(
        client.read_price_data_for_feed(&feed).price.to_u128(),
        Some(960_000u128)
    );
}

#[test]
fn only_the_owner_can_set_max_cluster_spread_bps() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 3, 2);
    let intruder = Address::generate(&env);

    env.mock_auths(&[MockAuth {
        address: &intruder,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_max_cluster_spread_bps",
            args: (500u32,).into_val(&env),
            sub_invokes: &[],
        },
    }]);
    let refused = catch_unwind(AssertUnwindSafe(|| {
        client.set_max_cluster_spread_bps(&500u32)
    }))
    .expect_err("a non-owner call must be refused");
    let message = refused
        .downcast_ref::<std::string::String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains("Error(Auth, InvalidAction)"),
        "expected a host Auth error, got: {message}"
    );
    assert_eq!(client.max_cluster_spread_bps(), 200);

    env.mock_all_auths();
    client.set_max_cluster_spread_bps(&500u32);
    assert_eq!(client.max_cluster_spread_bps(), 500);
}
