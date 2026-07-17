#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle_adapter::Error;

use ::common::oracle::providers::redstone::REDSTONE_DECIMALS;
use ::common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::{Env, String, Symbol};

#[test]
fn add_feed_and_remove_feed_maintain_asset_index() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);

    assert_eq!(client.assets().len(), 0);

    client.add_feed(&feed_id(&env), &asset);
    let assets = client.assets();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets.get(0).unwrap(), asset);

    client.remove_feed(&asset);
    assert_eq!(client.assets().len(), 0);
}

#[test]
fn add_feed_rejects_duplicate_mapping() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);

    client.add_feed(&feed_id(&env), &asset);
    let err = client.try_add_feed(&feed_id(&env), &asset);
    assert_eq!(err, Err(Ok(Error::FeedAlreadyMapped)));
}

#[test]
fn remove_feed_rejects_unmapped_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);

    let err = client.try_remove_feed(&asset);
    assert_eq!(err, Err(Ok(Error::FeedNotMapped)));
}

#[test]
fn lastprice_returns_none_for_unmapped_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);

    assert!(client.lastprice(&xlm_asset(&env)).is_none());
}

#[test]
fn lastprice_returns_none_when_no_aggregate_yet() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);

    // Feed is mapped, but no submission has ever reached this feed, so
    // `read_price_data_for_feed` internally returns `NoDataForFeed`, which
    // `lastprice` swallows into `None` per SEP-40 semantics.
    client.add_feed(&feed_id(&env), &asset);
    assert!(client.lastprice(&asset).is_none());
}

#[test]
fn lastprice_converts_price_and_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    let now = env.ledger().timestamp();
    let package_ts_ms = now * 1000;
    client.submit_price(&signers[0], &feed_id(&env), &12_345_678i128, &package_ts_ms);

    let data = client.lastprice(&asset).expect("expected price data");
    assert_eq!(data.price, 12_345_678i128);
    assert_eq!(data.timestamp, now);
}

#[test]
fn lastprice_exposes_observation_time_not_write_time() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    // A package observed 300s before it is submitted (still within the 900s
    // inclusion window, so it is aggregated). The aggregate's write time is
    // `now`, but its observation time is `now - 300`.
    advance_ledger_seconds(&env, 100_000);
    let now = env.ledger().timestamp();
    let observed_at = now - 300;
    client.submit_price(
        &signers[0],
        &feed_id(&env),
        &12_345_678i128,
        &(observed_at * 1000),
    );

    // SEP-40 must expose the observation time so downstream freshness checks
    // see the true age, not the ~now write time (which would look fresh).
    let data = client.lastprice(&asset).expect("expected price data");
    assert_eq!(data.timestamp, observed_at);
    assert_ne!(data.timestamp, now);
}

#[test]
fn decimals_always_equals_redstone_decimals() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    assert_eq!(
        client.decimals(),
        REDSTONE_DECIMALS
    );
}

#[test]
fn base_returns_usd() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    assert_eq!(
        client.base(),
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    );
}

#[test]
fn resolution_returns_stored_value() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    assert_eq!(client.resolution(), TEST_RESOLUTION);
}

#[test]
fn prices_returns_history_newest_first() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    // Advance a full `resolution` between submissions so each lands in a
    // distinct history bucket (sub-resolution submits overwrite in place).
    let start = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(start * 1000));
    advance_ledger_seconds(&env, TEST_RESOLUTION as u64);
    let t2 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t2 * 1000));

    let prices = client.prices(&asset, &12).expect("expected history");
    assert_eq!(prices.len(), 2);
    // `read_price_history` is newest-first.
    assert_eq!(prices.get(0).unwrap().price, 200i128);
    assert_eq!(prices.get(1).unwrap().price, 100i128);
}

#[test]
fn prices_returns_none_for_unmapped_asset() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    assert!(client.prices(&xlm_asset(&env), &12).is_none());
}

#[test]
fn price_finds_closest_sample_at_or_before_timestamp() {
    let env = Env::default();
    env.mock_all_auths();
    // Start from a non-zero ledger time so "before the earliest sample" is
    // a real, distinct timestamp rather than saturating at 0.
    advance_ledger_seconds(&env, 1000);
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    // A full `resolution` apart so each is its own history bucket.
    let step = TEST_RESOLUTION as u64;
    let t1 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t1 * 1000));
    advance_ledger_seconds(&env, step);
    let t2 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t2 * 1000));
    advance_ledger_seconds(&env, step);
    let t3 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &300i128, &(t3 * 1000));

    // Query strictly between t1 and t2: closest sample at or before is t1.
    let data = client.price(&asset, &(t1 + 30)).expect("expected sample");
    assert_eq!(data.price, 100i128);

    // Query at exactly t2: should match t2 itself, not t1 or t3.
    let data = client.price(&asset, &t2).expect("expected sample");
    assert_eq!(data.price, 200i128);

    // Query before the earliest sample: nothing qualifies.
    assert!(client.price(&asset, &(t1 - 1)).is_none());
}

#[test]
fn price_selects_closest_when_history_is_non_monotonic() {
    // History is ordered by recording; `package_timestamp` (the median's oldest
    // contributor) can move backwards when a signer joins with an older-but-
    // fresh observation. `price()` must scan for the greatest qualifying
    // observation, not trust newest-recorded position.
    let env = Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 5_000);
    let (client, _admin, signers) = setup(&env, 3, 2);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);
    // Disable bucketing so both recomputes retain distinct samples.
    client.set_resolution(&0);

    // Recompute 1 (recorded first): A,B at obs=now -> package_timestamp = now.
    let t_early = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t_early * 1000));
    client.submit_price(&signers[1], &feed_id(&env), &200i128, &(t_early * 1000));
    // median([100,200]) = 150 at package_timestamp t_early.

    // Recompute 2 (recorded later): C joins with an OLDER observation, so the
    // new sample's package_timestamp is smaller than the earlier sample's.
    advance_ledger_seconds(&env, 100);
    let older_obs = t_early - 50;
    client.submit_price(&signers[2], &feed_id(&env), &160i128, &(older_obs * 1000));
    // median([100,200,160]) = 160 at package_timestamp older_obs (< t_early).

    // Query at t_early: both samples qualify (older_obs < t_early <= t_early).
    // Closest at-or-before is the t_early sample (150), NOT the later-recorded
    // older_obs sample (160) that a naive first-match scan would return.
    let data = client.price(&asset, &t_early).expect("expected sample");
    assert_eq!(data.price, 150i128);
}

#[test]
fn price_prefers_newest_recorded_sample_on_equal_observation_time() {
    let env = Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 1_000);
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);
    // Disable bucketing so both recomputes retain distinct history samples.
    client.set_resolution(&0);

    // Two samples with the SAME observation time: the signer re-submits a
    // corrected price for the same package timestamp. History (newest-first)
    // holds [(200, t), (100, t)].
    let t = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t * 1000));
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t * 1000));

    // On an observation-time tie the scan must keep the first (newest
    // recorded) qualifying sample — the correction, not the superseded price.
    let data = client.price(&asset, &t).expect("expected sample");
    assert_eq!(data.price, 200i128);
}

#[test]
fn remove_feed_swap_moves_last_asset_into_gap() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    let asset_a = xlm_asset(&env);
    let asset_b = ReflectorAsset::Other(Symbol::new(&env, "BTC"));
    client.add_feed(&feed_id(&env), &asset_a);
    client.add_feed(&String::from_str(&env, "BTC/USD"), &asset_b);
    assert_eq!(client.assets().len(), 2);

    // Removing the FIRST asset swaps the last slot into the gap, so the index
    // stays gap-free and the survivor is still enumerable.
    client.remove_feed(&asset_a);
    let assets = client.assets();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets.get(0).unwrap(), asset_b);
}

#[test]
fn prices_returns_none_for_zero_records() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &1_000u64);

    // History exists, but zero records requested yields an empty slice -> None.
    assert!(client.prices(&asset, &0u32).is_none());
}
