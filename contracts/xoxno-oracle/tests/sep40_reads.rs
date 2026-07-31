#![cfg(test)]
extern crate std;

mod common;
use common::*;

use xoxno_oracle::Error;

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

    advance_ledger_seconds(&env, 100_000);
    let now = env.ledger().timestamp();
    let observed_at = now - 300;
    client.submit_price(
        &signers[0],
        &feed_id(&env),
        &12_345_678i128,
        &(observed_at * 1000),
    );

    let data = client.lastprice(&asset).expect("expected price data");
    assert_eq!(data.timestamp, observed_at);
    assert_ne!(data.timestamp, now);
}

#[test]
fn decimals_always_equals_redstone_decimals() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env, 1, 1);
    assert_eq!(client.decimals(), REDSTONE_DECIMALS);
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

    let start = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(start * 1000));
    advance_ledger_seconds(&env, TEST_RESOLUTION as u64);
    let t2 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t2 * 1000));

    let prices = client.prices(&asset, &12).expect("expected history");
    assert_eq!(prices.len(), 2);

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

    advance_ledger_seconds(&env, 1000);
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    let step = TEST_RESOLUTION as u64;
    let t1 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t1 * 1000));
    advance_ledger_seconds(&env, step);
    let t2 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t2 * 1000));
    advance_ledger_seconds(&env, step);
    let t3 = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &300i128, &(t3 * 1000));

    let data = client.price(&asset, &(t1 + 30)).expect("expected sample");
    assert_eq!(data.price, 100i128);

    let data = client.price(&asset, &t2).expect("expected sample");
    assert_eq!(data.price, 200i128);

    assert!(client.price(&asset, &(t1 - 1)).is_none());
}

#[test]
fn price_selects_closest_when_history_is_non_monotonic() {
    let env = Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 5_000);
    let (client, _admin, signers) = setup(&env, 3, 2);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    client.set_resolution(&0);

    let t_early = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t_early * 1000));
    client.submit_price(&signers[1], &feed_id(&env), &200i128, &(t_early * 1000));

    advance_ledger_seconds(&env, 100);
    let older_obs = t_early - 50;
    client.submit_price(&signers[2], &feed_id(&env), &160i128, &(older_obs * 1000));

    let data = client.price(&asset, &t_early).expect("expected sample");
    assert_eq!(data.price, 100i128);
}

#[test]
fn price_prefers_newest_recorded_sample_on_equal_observation_time() {
    let env = Env::default();
    env.mock_all_auths();
    advance_ledger_seconds(&env, 1_000);
    let (client, _admin, signers) = setup(&env, 1, 1);
    let asset = xlm_asset(&env);
    client.add_feed(&feed_id(&env), &asset);

    client.set_resolution(&0);

    let t = env.ledger().timestamp();
    client.submit_price(&signers[0], &feed_id(&env), &100i128, &(t * 1000));
    client.submit_price(&signers[0], &feed_id(&env), &200i128, &(t * 1000));

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

    assert!(client.prices(&asset, &0u32).is_none());
}
