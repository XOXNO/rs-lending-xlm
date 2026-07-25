//! `compose()` gathers every leg's outcome in one traversal without
//! panicking. These tests pin `Composition`'s shape directly (no contract
//! client, no `price`/`status` rendering) so Tasks 4 and 5 can build on a
//! settled contract.

use super::*;
use common::constants::WAD;
use common::types::{OracleSourceConfigOption, OracleTolerance, RedStoneSourceConfig};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{Address, String};

fn register_redstone_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
    let id = env.register(MockRedStonePriceFeed, ());
    (id.clone(), MockRedStonePriceFeedClient::new(env, &id))
}

fn redstone_single(env: &Env, feed: &Address, feed_id: &str, max_stale: u64) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_000,
            lower_ratio_bps: 10_000,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, feed_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

fn redstone_dual(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    anchor_id: &str,
    max_stale: u64,
    upper_bps: u32,
    lower_bps: u32,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        tolerance: OracleTolerance {
            upper_ratio_bps: upper_bps,
            lower_ratio_bps: lower_bps,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: feed.clone(),
            feed_id: String::from_str(env, primary_id),
            decimals: 8,
            max_stale_seconds: max_stale,
        }),
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::RedStone(
            RedStoneSourceConfig {
                contract: feed.clone(),
                feed_id: String::from_str(env, anchor_id),
                decimals: 8,
                max_stale_seconds: max_stale,
            },
        )),
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: WAD * 2,
    }
}

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

/// Dual strategy, both legs readable and inside the tolerance band: `blended`
/// returns the midpoint and the older of the two timestamps.
#[test]
fn dual_strategy_both_legs_in_band_blends_to_midpoint() {
    let env = Env::default();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| li.timestamp = now);
    let mut cache = ResolutionContext::new(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &WAD);
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    let composition = compose(&mut cache, &config);

    assert!(composition.primary.result.is_ok());
    let anchor_leg = composition.anchor.as_ref().expect("anchor leg present");
    assert!(anchor_leg.result.is_ok());
    assert!(!composition.dual_missing_anchor);

    let blended = composition
        .blended(&env, &config.tolerance)
        .expect("dual in-band blend");
    assert_eq!(blended, (WAD, now));
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
