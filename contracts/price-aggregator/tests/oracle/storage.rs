//! `AssetOracle` persistence: what was written comes back whole, what was never
//! written comes back as `None`, and a read renews the entry's shared-tier TTL.
//!
//! Absence is the interesting half. Every caller above this layer treats a
//! missing config as "not configured" and fails closed on it; a default-shaped
//! `AssetOracleConfig` handed back instead would be a live oracle with a zeroed
//! sanity band, so `None` is a contract, not an implementation detail.
//!
//! Persistent storage is only reachable from inside a contract frame, so every
//! case runs through `in_contract`.

use super::*;
use crate::test_support::{in_contract, redstone_single, register_redstone_feed};
use crate::PriceAggregator;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};

/// A stored config comes back field-for-field. Compared as a whole struct
/// rather than field by field, so a codec that silently drops or reorders a
/// field cannot pass.
#[test]
fn set_then_get_round_trips_the_whole_config() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);

    let read_back = in_contract(&env, || {
        set_oracle_config(&env, &asset, &config);
        get_oracle_config(&env, &asset)
    });

    assert_eq!(read_back, Some(config));
}

/// An asset that was never configured reads as `None` — not a zeroed
/// `AssetOracleConfig`, which would present as a configured oracle whose sanity
/// band is disabled.
#[test]
fn missing_config_reads_as_none() {
    let env = Env::default();
    let asset = Address::generate(&env);

    let read_back = in_contract(&env, || get_oracle_config(&env, &asset));

    assert!(read_back.is_none());
}

/// Entries are keyed per asset: two assets configured in the same contract keep
/// their own configs, and a third that was never written stays `None` even once
/// the store is non-empty.
#[test]
fn configs_are_keyed_per_asset() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let first = Address::generate(&env);
    let second = Address::generate(&env);
    let unwritten = Address::generate(&env);
    let first_config = redstone_single(&env, &feed, "BTC/USD", 900);
    let second_config = redstone_single(&env, &feed, "ETH/USD", 60);

    let (read_first, read_second, read_unwritten) = in_contract(&env, || {
        set_oracle_config(&env, &first, &first_config);
        set_oracle_config(&env, &second, &second_config);
        (
            get_oracle_config(&env, &first),
            get_oracle_config(&env, &second),
            get_oracle_config(&env, &unwritten),
        )
    });

    assert_eq!(read_first, Some(first_config));
    assert_eq!(read_second, Some(second_config));
    assert!(read_unwritten.is_none());
}

/// A second write replaces the first. The setter is the only way governance
/// re-points an oracle, so a write that skipped an occupied key would strand
/// the old source.
#[test]
fn set_replaces_the_previous_config() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let original = redstone_single(&env, &feed, "BTC/USD", 900);
    let replacement = redstone_single(&env, &feed, "ETH/USD", 60);

    let read_back = in_contract(&env, || {
        set_oracle_config(&env, &asset, &original);
        set_oracle_config(&env, &asset, &replacement);
        get_oracle_config(&env, &asset)
    });

    assert_eq!(read_back, Some(replacement));
}

/// Removal returns the asset to the unconfigured state, not to a stale copy of
/// what was there.
#[test]
fn remove_returns_the_asset_to_unconfigured() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);

    let (before, after) = in_contract(&env, || {
        set_oracle_config(&env, &asset, &config);
        let before = get_oracle_config(&env, &asset);
        remove_oracle_config(&env, &asset);
        (before, get_oracle_config(&env, &asset))
    });

    assert!(before.is_some());
    assert!(after.is_none());
}

/// A read renews the entry's TTL. Oracle configs are written once at listing
/// and then only read, so the read path is the only thing standing between a
/// live market and an archived config; the ledger is advanced far enough that
/// the entry drops under `TTL_THRESHOLD_SHARED` before the read.
#[test]
fn get_renews_the_shared_tier_ttl() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);
    let id = env.register(PriceAggregator, (Address::generate(&env),));
    let key = AggregatorKey::AssetOracle(asset.clone());

    env.as_contract(&id, || set_oracle_config(&env, &asset, &config));
    // Burn all but a sliver of the bump, so the read has something to renew.
    let elapsed = TTL_BUMP_SHARED - TTL_THRESHOLD_SHARED / 2;
    env.ledger().with_mut(|li| li.sequence_number += elapsed);

    let (before, after) = env.as_contract(&id, || {
        let before = env.storage().persistent().get_ttl(&key);
        assert!(get_oracle_config(&env, &asset).is_some());
        (before, env.storage().persistent().get_ttl(&key))
    });

    assert_eq!(before, TTL_BUMP_SHARED - elapsed);
    assert_eq!(after, TTL_BUMP_SHARED);
}
