//! Contract-surface tests: ownership, config setters, fail-closed pricing,
//! soft status flags, and end-to-end reads through a RedStone mock.

use common::types::{
    AssetOracleConfig, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy,
    OracleTolerance, RedStoneSourceConfig,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use price_aggregator::{PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, String, Vec};

const WAD: i128 = 1_000_000_000_000_000_000;

fn register_agg(env: &Env) -> (Address, PriceAggregatorClient<'_>) {
    let owner = Address::generate(env);
    let id = env.register(PriceAggregator, (owner.clone(),));
    (owner, PriceAggregatorClient::new(env, &id))
}

fn register_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
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
        // Single-source band must stay within ±10% midpoint-relative.
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
        // Anchored configs allow a wider sanity window.
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: WAD * 2,
    }
}

#[test]
fn set_oracle_config_roundtrips_through_storage() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _) = register_feed(&env);
    let cfg = redstone_single(&env, &feed, "BTC/USD", 900);

    client.set_oracle_config(&asset, &cfg);
    assert_eq!(client.oracle_config(&asset), Some(cfg));
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn prices_reverts_for_unconfigured_asset() {
    let env = Env::default();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    client.prices(&Vec::from_array(&env, [asset]));
}

#[test]
fn price_and_prices_resolve_live_redstone_feed() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let feed_id = String::from_str(&env, "BTC/USD");
    feed_client.set_price(&feed_id, &WAD);
    client.set_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

    let single = client.price(&asset);
    assert_eq!(single.price_wad, WAD);
    assert_eq!(single.asset_decimals, 7);

    let bulk = client.prices(&Vec::from_array(&env, [asset.clone()]));
    assert_eq!(bulk.get(asset).unwrap().price_wad, WAD);
}

#[test]
fn price_status_and_prices_status_report_valid_single() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.set_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

    let status = client.price_status(&asset);
    assert!(status.valid);
    assert!(!status.stale);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, WAD);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, WAD);
    assert!(status.price_timestamp > 0);

    let bulk = client.prices_status(&Vec::from_array(&env, [asset.clone()]));
    assert!(bulk.get(asset).unwrap().valid);
}

#[test]
fn price_status_unconfigured_is_unusable() {
    let env = Env::default();
    let (_owner, client) = register_agg(&env);
    let status = client.price_status(&Address::generate(&env));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
    assert_eq!(status.price_timestamp, 0);
}

#[test]
fn price_status_pending_config_is_unusable() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    client.seed_oracle_config(&asset, &AssetOracleConfig::pending_for(asset.clone(), 7));

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn price_status_missing_primary_feed_is_unusable() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _) = register_feed(&env);
    // Config points at a feed that was never set.
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "MISSING", 900));

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn price_status_marks_stale_single_source() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 60));

    // Advance well past max_stale_seconds.
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10_000;
    });

    let status = client.price_status(&asset);
    assert!(status.stale);
    assert!(!status.valid);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, WAD);
}

#[test]
fn price_status_dual_without_anchor_marks_deviation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.strategy = OracleStrategy::PrimaryWithAnchor;
    cfg.anchor = OracleSourceConfigOption::None;
    client.seed_oracle_config(&asset, &cfg);

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert!(status.deviation);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, 0);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn price_status_dual_missing_anchor_feed_marks_deviation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    // Anchor feed never set.
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert!(status.deviation);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, 0);
}

#[test]
fn price_status_dual_in_band_is_valid_midpoint() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD + WAD / 100)); // +1%
    client.set_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);
    assert!(status.valid);
    assert!(!status.stale);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, (WAD + (WAD + WAD / 100)) / 2);
}

#[test]
fn price_status_dual_out_of_band_marks_deviation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD * 2));
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert!(status.deviation);
    assert!(!status.stale);
    // Midpoint still surfaced for diagnostics.
    assert_eq!(status.final_wad, (WAD + WAD * 2) / 2);
}

#[test]
fn price_status_outside_sanity_band_is_invalid() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    // Live price is WAD; band is far above it.
    cfg.min_sanity_price_wad = WAD * 10;
    cfg.max_sanity_price_wad = WAD * 20;
    client.seed_oracle_config(&asset, &cfg);

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert!(!status.stale);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, WAD);
}

#[test]
fn set_sanity_band_and_tolerance_update_live_config() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.set_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

    // Walk band while still containing live price (±5% stays Single-safe).
    client.set_sanity_band(&asset, &(WAD - WAD / 20), &(WAD + WAD / 20));
    let after_band = client.oracle_config(&asset).unwrap();
    assert_eq!(after_band.min_sanity_price_wad, WAD - WAD / 20);
    assert_eq!(after_band.max_sanity_price_wad, WAD + WAD / 20);

    let tol = OracleTolerance {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_800,
    };
    client.set_tolerance(&asset, &tol);
    assert_eq!(client.oracle_config(&asset).unwrap().tolerance, tol);
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn set_tolerance_unknown_asset_reverts_oracle_not_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    client.set_tolerance(
        &Address::generate(&env),
        &OracleTolerance {
            upper_ratio_bps: 10_200,
            lower_ratio_bps: 9_800,
        },
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn set_sanity_band_unknown_asset_reverts_oracle_not_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    client.set_sanity_band(&Address::generate(&env), &(WAD / 2), &(WAD * 2));
}

#[test]
fn remove_oracle_config_disables_pricing() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));
    assert!(client.oracle_config(&asset).is_some());

    client.remove_oracle_config(&asset);
    assert!(client.oracle_config(&asset).is_none());
    assert!(!client.price_status(&asset).valid);
}

#[test]
fn ownable_get_owner_and_two_step_transfer() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 10;
        li.timestamp = 1_700_000_000;
    });
    let (owner, client) = register_agg(&env);
    assert_eq!(client.get_owner(), Some(owner));

    let new_owner = Address::generate(&env);
    // live_until_ledger must be in the future relative to sequence.
    client.transfer_ownership(&new_owner, &100u32);
    client.accept_ownership();
    assert_eq!(client.get_owner(), Some(new_owner));
}

#[test]
fn ownable_renounce_clears_owner() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    client.renounce_ownership();
    assert_eq!(client.get_owner(), None);
}

// ---------------------------------------------------------------------------
// Security audit extensions (hard path vs soft status, staleness ownership,
// live-price containment on set_sanity_band, dual hard revert).
// ---------------------------------------------------------------------------

/// H-ORC-SOFT: soft `price_status` reports stale without reverting; hard `price`
/// reverts `PriceFeedStale` so write-path consumers cannot soft-accept.
#[test]
fn audit_hard_price_reverts_stale_while_status_soft_flags() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 60));

    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10_000;
    });

    let status = client.price_status(&asset);
    assert!(status.stale);
    assert!(!status.valid);
    assert_eq!(status.final_wad, WAD);

    // Hard path must fail closed (not return the stale WAD).
    let hard = client.try_price(&asset);
    assert!(hard.is_err(), "hard price must revert on stale feed; got {hard:?}");
}

/// H-ORC-DUAL-HARD: dual-source out-of-band reverts hard `price` with
/// `UnsafePriceNotAllowed` while soft status only sets deviation.
#[test]
fn audit_hard_price_reverts_dual_out_of_band() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD * 2));
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert!(status.deviation);

    let hard = client.try_price(&asset);
    assert!(
        hard.is_err(),
        "hard dual out-of-band must revert; got {hard:?}"
    );
}

/// H-ORC-STALE-OWNERSHIP: multi-feed freshness uses the **source**
/// `max_stale_seconds`, not `AssetOracleConfig.max_price_stale_seconds`.
/// A short market max with a long source max still accepts the observation
/// (ops footgun / design residual if operators mis-set only the market field).
#[test]
fn audit_multi_feed_stale_uses_source_max_not_market_max() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    // Market default 30s, source allows 900s.
    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.max_price_stale_seconds = 30;
    if let OracleSourceConfig::RedStone(ref mut s) = cfg.primary {
        s.max_stale_seconds = 900;
    }
    client.seed_oracle_config(&asset, &cfg);

    // Age 100s: past market 30s, inside source 900s → hard price still succeeds.
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 100;
    });
    let feed_out = client.price(&asset);
    assert_eq!(
        feed_out.price_wad, WAD,
        "H-ORC-STALE-OWNERSHIP: multi-feed must key freshness on source max_stale, not market max"
    );
}

/// H-ORC-SANITY-CONTAIN: `set_sanity_band` rejects a band that excludes the live
/// price (live-price containment probe via `resolve_with_config`).
#[test]
fn audit_set_sanity_band_rejects_band_excluding_live_price() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    // Single source with a band that still contains live WAD (±5%).
    client.set_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));
    // Overlap old band but exclude live WAD entirely above the print.
    // Containment probe must fail closed before storage write.
    let result = client.try_set_sanity_band(&asset, &(WAD + WAD / 100), &(WAD + WAD / 20));
    assert!(
        result.is_err(),
        "set_sanity_band must reject a band that excludes live price; got {result:?}"
    );
    // Config must remain the pre-call band (no partial write).
    let cfg = client.oracle_config(&asset).unwrap();
    assert_eq!(cfg.min_sanity_price_wad, WAD - WAD / 20);
    assert_eq!(cfg.max_sanity_price_wad, WAD + WAD / 20);
}

/// H-ORC-MIDPOINT: in-band dual hard path returns integer midpoint (not primary alone).
#[test]
fn audit_hard_price_dual_in_band_is_midpoint() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let primary = WAD;
    let anchor = WAD + WAD / 50; // +2%
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &primary);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &anchor);
    client.set_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let hard = client.price(&asset);
    assert_eq!(hard.price_wad, (primary + anchor) / 2);
}

/// H-ORC-ZERO: zero primary price fails closed on hard path.
#[test]
fn audit_hard_price_rejects_zero_primary() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    // set_price scales WAD; use set_price_data with raw zero.
    feed_client.set_price_data(
        &String::from_str(&env, "BTC/USD"),
        &0i128,
        &(1_700_000_000u64 * 1000),
        &(1_700_000_000u64 * 1000),
    );
    client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

    let hard = client.try_price(&asset);
    assert!(hard.is_err(), "zero price must fail closed; got {hard:?}");
}

/// H-ORC-CFG-NO-PROBE: `set_oracle_config` validates structure/bands but does
/// **not** require a live readable feed. Operators can store a config that
/// immediately fails closed on `price` until the feed is populated (ops residual).
#[test]
fn audit_set_oracle_config_allows_missing_live_feed() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _) = register_feed(&env);
    // Feed never set — config still stores.
    client.set_oracle_config(&asset, &redstone_single(&env, &feed, "MISSING", 900));
    assert!(client.oracle_config(&asset).is_some());
    let hard = client.try_price(&asset);
    assert!(
        hard.is_err(),
        "missing feed must fail closed on hard price after config store; got {hard:?}"
    );
}

/// H-ORC-SANITY-HARD: hard `price` reverts when final is outside sanity band
/// (status only soft-flags invalid).
#[test]
fn audit_hard_price_reverts_outside_sanity_band() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.min_sanity_price_wad = WAD * 10;
    cfg.max_sanity_price_wad = WAD * 20;
    client.seed_oracle_config(&asset, &cfg);

    let status = client.price_status(&asset);
    assert!(!status.valid);
    assert_eq!(status.final_wad, WAD);

    let hard = client.try_price(&asset);
    assert!(
        hard.is_err(),
        "hard price must revert outside sanity band; got {hard:?}"
    );
}

