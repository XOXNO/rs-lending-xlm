//! `compose()` gathers every leg's outcome in one traversal without
//! panicking. These tests pin `Composition`'s shape directly (no contract
//! client, no `price`/`status` rendering) so Tasks 4 and 5 can build on a
//! settled contract.

use super::*;
use common::constants::WAD;
use common::types::OracleSourceConfigOption;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String};

use crate::test_support::{
    redstone_dual, redstone_single, reflector_single, register_redstone_feed, EmptyReflector,
};

/// Single strategy with a readable primary: no anchor leg, `blended` mirrors
/// the primary's price and timestamp verbatim.
#[test]
fn single_strategy_readable_leg_blends_to_primary() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);

    let composition = compose(&mut cache, &config);

    let primary = composition
        .primary
        .result
        .as_ref()
        .expect("primary readable");
    assert_eq!(primary.price_wad, WAD);
    assert_eq!(primary.timestamp(), now);
    assert!(!composition.primary.stale);
    assert!(composition.anchor.is_none());
    assert!(!composition.dual_missing_anchor);

    let blended = composition
        .blended(&env, &config.tolerance)
        .expect("single-leg blend");
    assert_eq!(blended, (WAD, now));
}

/// Dual strategy, both legs readable, inside the tolerance band, and priced
/// *differently*: `blended` returns the actual midpoint of the two distinct
/// prices, not just the primary echoed back (which identical inputs could
/// not distinguish from a bug that skips averaging entirely).
#[test]
fn dual_strategy_both_legs_in_band_blends_to_midpoint() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    let primary_price = WAD;
    let anchor_price = WAD + WAD / 50; // +2%, inside the configured ±5% band.
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &primary_price);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &anchor_price);
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    let composition = compose(&mut cache, &config);

    assert!(composition.primary.result.is_ok());
    let anchor_leg = composition.anchor.as_ref().expect("anchor leg present");
    assert!(anchor_leg.result.is_ok());
    assert!(!composition.dual_missing_anchor);

    let blended = composition
        .blended(&env, &config.tolerance)
        .expect("dual in-band blend");
    assert_eq!(blended, ((primary_price + anchor_price) / 2, now));
}

/// Dual strategy, both legs readable, but the primary leg is the older of the
/// two: `blended`'s timestamp is the primary's, not the anchor's.
#[test]
fn dual_strategy_primary_older_blends_to_primary_timestamp() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    let older = now - 500;
    feed_client.set_price_data(
        &String::from_str(&env, "PRIMARY"),
        &WAD,
        &(older * 1_000),
        &(older * 1_000),
    );
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &WAD);
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    let composition = compose(&mut cache, &config);

    let blended = composition
        .blended(&env, &config.tolerance)
        .expect("dual in-band blend");
    assert_eq!(blended, (WAD, older));
}

/// Dual strategy, both legs readable, but the anchor leg is the older of the
/// two: `blended`'s timestamp is the anchor's, not the primary's. Pairs with
/// the primary-older case so the older-of-two-legs rule cannot pass by
/// always returning whichever leg happens to be read first.
#[test]
fn dual_strategy_anchor_older_blends_to_anchor_timestamp() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    let older = now - 500;
    feed_client.set_price_data(
        &String::from_str(&env, "ANCHOR"),
        &WAD,
        &(older * 1_000),
        &(older * 1_000),
    );
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    let composition = compose(&mut cache, &config);

    let blended = composition
        .blended(&env, &config.tolerance)
        .expect("dual in-band blend");
    assert_eq!(blended, (WAD, older));
}

/// Dual strategy but the config carries no anchor source: `compose` cannot
/// read a leg that does not exist, so it flags `dual_missing_anchor` instead
/// of guessing at an error. `blended` reports no price.
#[test]
fn dual_strategy_missing_anchor_config_sets_flag() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut config = redstone_single(&env, &feed, "BTC/USD", 900);
    config.strategy = OracleStrategy::PrimaryWithAnchor;
    config.anchor = OracleSourceConfigOption::None;

    let composition = compose(&mut cache, &config);

    assert!(composition.primary.result.is_ok());
    assert!(composition.anchor.is_none());
    assert!(composition.dual_missing_anchor);
    assert!(composition.blended(&env, &config.tolerance).is_none());
}

/// Dual strategy, anchor configured but unreadable (feed id never populated):
/// the anchor leg carries the provider family that would have raised the
/// hard-path error, and `blended` reports no price rather than panicking.
#[test]
fn dual_strategy_anchor_unreadable_carries_source_kind() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    // "ANCHOR" feed id is never populated on the mock.
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    let composition = compose(&mut cache, &config);

    assert!(composition.primary.result.is_ok());
    let anchor_leg = composition.anchor.as_ref().expect("anchor leg present");
    assert_eq!(
        anchor_leg.result.as_ref().unwrap_err(),
        &SourceKind::MultiFeed
    );
    assert!(!composition.dual_missing_anchor);
    assert!(composition.blended(&env, &config.tolerance).is_none());
}

/// Primary unreadable (feed id never populated): `blended`'s early return
/// (`self.primary.result.as_ref().ok()?`) fires before the anchor is ever
/// consulted, so the composition reports no price.
#[test]
fn primary_unreadable_blended_returns_none() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, _feed_client) = register_redstone_feed(&env);
    // "MISSING" feed id is never populated on the mock.
    let config = redstone_single(&env, &feed, "MISSING", 900);

    let composition = compose(&mut cache, &config);

    assert_eq!(
        composition.primary.result.as_ref().unwrap_err(),
        &SourceKind::MultiFeed
    );
    assert!(composition.blended(&env, &config.tolerance).is_none());
}

/// Primary unreadable via a Reflector source: the leg carries
/// `SourceKind::Reflector`, not the `MultiFeed` family covered by the
/// RedStone/Xoxno unreadable cases above. Task 4 branches on this
/// discriminant to choose `NoLastPrice` vs `InvalidTicker`, so a wrong
/// mapping here is an ABI break.
#[test]
fn primary_reflector_unreadable_carries_source_kind() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let config = reflector_single(&reflector, &asset, 900);

    let composition = compose(&mut cache, &config);

    assert_eq!(
        composition.primary.result.as_ref().unwrap_err(),
        &SourceKind::Reflector
    );
    assert!(composition.blended(&env, &config.tolerance).is_none());
}

/// A leg past its max-stale window is still `Ok`: `compose` never panics on
/// staleness, it only flags it. `blended` does not consult `stale` — that
/// decision belongs to the renderer, not the traversal.
#[test]
fn stale_leg_stays_ok_with_stale_flag_set() {
    let env = Env::default();
    let now: u64 = 1_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    let config = redstone_single(&env, &feed, "BTC/USD", 60);

    // Advance well past max_stale_seconds. `ResolutionContext` snapshots the
    // ledger timestamp at construction, so it is built after the advance.
    env.ledger().with_mut(|li| li.timestamp = now + 10_000);
    let mut cache = ResolutionContext::new(&env);

    let composition = compose(&mut cache, &config);

    assert!(composition.primary.result.is_ok());
    assert!(composition.primary.stale);
    // Readability, not staleness, drives `blended`.
    assert!(composition.blended(&env, &config.tolerance).is_some());
}
