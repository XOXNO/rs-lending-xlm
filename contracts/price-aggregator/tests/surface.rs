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
