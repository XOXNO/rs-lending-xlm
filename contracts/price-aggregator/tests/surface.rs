//! Contract-surface tests: ownership, config setters, fail-closed pricing,
//! soft status flags, and end-to-end reads through a RedStone mock.

use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance,
    PriceKey, PriceSource, ProviderKind, ProviderRef, TrustDomain,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use price_aggregator::{Error, PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger as _};
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

/// One RedStone feed. Declared [`FeedNature::Fundamental`] so a lone source
/// satisfies the smoothing rule: a published price is not moved by trading, so
/// it needs no window to defend it.
fn redstone_single(env: &Env, feed: &Address, feed_id: &str, max_stale: u64) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        sources: soroban_sdk::vec![env, redstone_feed(env, feed, feed_id, max_stale)],
        // Inert on a single source, but still validated on write: a stored
        // `10_000/10_000` would read as "±0% tolerance", an assertion the
        // config has no business making. `set_tolerance` holds the same rule,
        // so accepting it here would leave the two write paths disagreeing.
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        // Single-source band must stay within ±10% midpoint-relative.
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

fn redstone_feed(env: &Env, feed: &Address, feed_id: &str, max_stale: u64) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: feed.clone(),
            feed_id: String::from_str(env, feed_id),
            kind: ProviderKind::RedStone,
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    })
}

/// Two RedStone feeds on **one** adapter.
///
/// That is shared trust, so it is declared rather than hidden: `RequireDisjoint`
/// would (correctly) reject it, since whoever controls that adapter controls
/// both legs and the agreement band would compare a value against itself. The
/// waiver is what makes the sharing visible to a reviewer.
fn redstone_dual(
    env: &Env,
    feed: &Address,
    primary_id: &str,
    anchor_id: &str,
    max_stale: u64,
    upper_bps: u32,
    lower_bps: u32,
) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        sources: soroban_sdk::vec![
            env,
            redstone_feed(env, feed, primary_id, max_stale),
            redstone_feed(env, feed, anchor_id, max_stale),
        ],
        tolerance: OracleTolerance {
            upper_ratio_bps: upper_bps,
            lower_ratio_bps: lower_bps,
        },
        independence: IndependencePolicy::AllowShared(soroban_sdk::vec![
            env,
            TrustDomain {
                kind: ProviderKind::RedStone,
                contract: feed.clone(),
            }
        ]),
        // Anchored configs allow a wider sanity window.
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: WAD * 2,
    }
}

#[test]
fn set_oracle_roundtrips_through_storage() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    // The write path resolves the config before storing it, so the feed has to
    // be live for a successful round trip.
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    let cfg = redstone_single(&env, &feed, "BTC/USD", 900);

    client.set_oracle(&PriceKey::Token(asset.clone()), &cfg);
    // Every config write publishes exactly one UpdateAssetOracleEvent.
    assert_eq!(env.events().all().events().len(), 1);
    assert_eq!(
        client.oracle(&PriceKey::Token(asset.clone())),
        Some(cfg)
    );
}

// Exactly one stale leg must mark the whole dual read stale (primary written
// far in the past, anchor fresh).
#[test]
fn quote_dual_one_stale_leg_marks_stale() {
    let env = Env::default();
    env.mock_all_auths();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| {
        li.timestamp = now;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let stale_ms = (now - 10_000) * 1_000;
    feed_client.set_price_data(
        &String::from_str(&env, "PRIMARY"),
        &WAD,
        &stale_ms,
        &stale_ms,
    );
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &WAD);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(status.stale);
    assert!(!status.valid);
    assert!(!status.deviation);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, WAD);
}

#[test]
fn prices_reverts_for_unconfigured_asset() {
    let env = Env::default();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    assert!(client
        .try_prices(&Vec::from_array(&env, [PriceKey::Token(asset.clone())]))
        .is_err());
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
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let single = client.price(&PriceKey::Token(asset.clone()));
    assert_eq!(single.price_wad, WAD);
    assert_eq!(single.asset_decimals, 7);

    let bulk = client.prices(&Vec::from_array(&env, [PriceKey::Token(asset.clone())]));
    assert_eq!(bulk.get(PriceKey::Token(asset.clone())).unwrap().price_wad, WAD);
}

#[test]
fn quote_and_quotes_report_valid_single() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(status.valid);
    assert!(!status.stale);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, WAD);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, WAD);
    assert!(status.price_timestamp > 0);

    let bulk = client.quotes(&Vec::from_array(&env, [PriceKey::Token(asset.clone())]));
    assert!(bulk.get(PriceKey::Token(asset.clone())).unwrap().valid);
}

#[test]
fn quote_unconfigured_is_unusable() {
    let env = Env::default();
    let (_owner, client) = register_agg(&env);
    let status = client.quote(&PriceKey::Token(Address::generate(&env)));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
    assert_eq!(status.price_timestamp, 0);
}

#[test]
fn quote_unconfigured_key_is_unusable() {
    // v1 stored a `pending_for` placeholder for a market awaiting its oracle.
    // The composable model has no placeholder: the key simply has no entry, and
    // the soft path must report that as unusable rather than revert.
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

// A present-but-invalid payload (non-positive price) must soften to unusable
// on the status path, not revert. The hard `price` path still reverts.
#[test]
fn quote_non_positive_payload_is_unusable_without_revert() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let feed_id = String::from_str(&env, "BTC/USD");
    feed_client.set_price(&feed_id, &0); // present feed, non-positive price
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);

    // Hard path fails closed on the same payload.
    assert!(client.try_price(&PriceKey::Token(asset.clone())).is_err());
}

// A future-timestamped payload also softens to unusable rather than reverting.
#[test]
fn quote_future_timestamp_is_unusable_without_revert() {
    let env = Env::default();
    env.mock_all_auths();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| {
        li.timestamp = now;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let feed_id = String::from_str(&env, "BTC/USD");
    // Package/write timestamps far beyond now + skew (ms).
    let future_ms = (now + 100_000) * 1_000;
    feed_client.set_price_data(&feed_id, &WAD, &future_ms, &future_ms);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);

    assert!(client.try_price(&PriceKey::Token(asset.clone())).is_err());
}

// Exactly ONE of the two multi-feed timestamps in the future is enough to
// mark the read unusable — pins the `package_future || write_future` guard
// (an `&&` would wrongly accept a payload with one future leg).
#[test]
fn quote_single_future_multi_feed_timestamp_is_unusable() {
    let env = Env::default();
    env.mock_all_auths();
    let now: u64 = 1_700_000_000;
    env.ledger().with_mut(|li| {
        li.timestamp = now;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let feed_id = String::from_str(&env, "BTC/USD");
    let valid_ms = now * 1_000;
    let future_ms = (now + 100_000) * 1_000;
    // package_timestamp future, write_timestamp valid.
    feed_client.set_price_data(&feed_id, &WAD, &future_ms, &valid_ms);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn quote_missing_primary_feed_is_unusable() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _) = register_feed(&env);
    // Config points at a feed that was never set.
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "MISSING", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn quote_marks_stale_single_source() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 60),
    );

    // Advance well past max_stale_seconds.
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10_000;
    });

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(status.stale);
    assert!(!status.valid);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, WAD);
}

#[test]
fn quote_sourceless_config_is_unusable() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);

    // Arity is enforced at configure time (`SourceCountOutOfRange`), so this
    // shape can only exist if it were written past validation. Seeded directly
    // to prove the read path softens rather than panicking on it.
    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.sources = Vec::new(&env);
    client.seed_oracle(&PriceKey::Token(asset.clone()), &cfg);

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.primary_wad, 0);
    assert_eq!(status.secondary_wad, 0);
    assert_eq!(status.final_wad, 0);
}

#[test]
fn quote_dual_missing_anchor_feed_marks_deviation() {
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
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert!(status.deviation);
    assert_eq!(status.primary_wad, WAD);
    assert_eq!(status.secondary_wad, 0);
}

#[test]
fn quote_dual_in_band_is_valid_midpoint() {
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
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(status.valid);
    assert!(!status.stale);
    assert!(!status.deviation);
    assert_eq!(status.final_wad, (WAD + (WAD + WAD / 100)) / 2);
}

#[test]
fn quote_dual_out_of_band_marks_deviation() {
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
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert!(status.deviation);
    assert!(!status.stale);
    // Midpoint still surfaced for diagnostics.
    assert_eq!(status.final_wad, (WAD + WAD * 2) / 2);
}

#[test]
fn quote_outside_sanity_band_is_invalid() {
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
    client.seed_oracle(&PriceKey::Token(asset.clone()), &cfg);

    let status = client.quote(&PriceKey::Token(asset.clone()));
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
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    // Walk band while still containing live price (±5% stays Single-safe).
    client.set_sanity_band(
        &PriceKey::Token(asset.clone()),
        &(WAD - WAD / 20),
        &(WAD + WAD / 20),
    );
    let after_band = client.oracle(&PriceKey::Token(asset.clone())).unwrap();
    assert_eq!(after_band.min_sanity_price_wad, WAD - WAD / 20);
    assert_eq!(after_band.max_sanity_price_wad, WAD + WAD / 20);

    // Reciprocal of 10_200 BPS: half-up(10_000² / 10_200) = 9_804.
    let tol = OracleTolerance {
        upper_ratio_bps: 10_200,
        lower_ratio_bps: 9_804,
    };
    client.set_tolerance(&PriceKey::Token(asset.clone()), &tol);
    assert_eq!(
        client
            .oracle(&PriceKey::Token(asset.clone()))
            .unwrap()
            .tolerance,
        tol
    );
}

#[test]
fn set_tolerance_unknown_asset_reverts_oracle_not_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    assert_eq!(
        client.try_set_tolerance(
            &PriceKey::Token(Address::generate(&env)),
            &OracleTolerance {
                upper_ratio_bps: 10_200,
                lower_ratio_bps: 9_804,
            },
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::OracleNotConfigured as u32,
        )))
    );
}

#[test]
fn set_tolerance_rejects_non_reciprocal_band() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    // Additive ±5% is envelope-valid but not reciprocal of upper.
    assert_eq!(
        client.try_set_tolerance(
            &PriceKey::Token(asset),
            &OracleTolerance {
                upper_ratio_bps: 10_500,
                lower_ratio_bps: 9_500,
            },
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::BadLastTolerance as u32,
        )))
    );
}

#[test]
fn set_tolerance_live_probe_rejects_dual_out_of_new_band() {
    // Widen/tighten with live probe: disagreeing legs under the *new* band
    // must not commit (same containment discipline as set_sanity_band).
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    // ~10% apart: inside max ±25% band, outside ±2% band.
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD + WAD / 10));
    // Max envelope reciprocal: upper 12_500 → lower 8_000.
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 12_500, 8_000),
    );

    // Tighten to ±2% reciprocal (10_200 / 9_804): live midpoint fails deviation.
    assert_eq!(
        client.try_set_tolerance(
            &PriceKey::Token(asset),
            &OracleTolerance {
                upper_ratio_bps: 10_200,
                lower_ratio_bps: 9_804,
            },
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::UnsafePriceNotAllowed as u32,
        )))
    );
}

#[test]
fn set_sanity_band_unknown_asset_reverts_oracle_not_configured() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    assert_eq!(
        client.try_set_sanity_band(
            &PriceKey::Token(Address::generate(&env)),
            &(WAD / 2),
            &(WAD * 2)
        ),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::OracleNotConfigured as u32,
        )))
    );
}

#[test]
fn remove_oracle_disables_pricing() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );
    assert!(client.oracle(&PriceKey::Token(asset.clone())).is_some());

    client.remove_oracle(&PriceKey::Token(asset.clone()));
    assert!(client.oracle(&PriceKey::Token(asset.clone())).is_none());
    assert!(!client.quote(&PriceKey::Token(asset.clone())).valid);
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

/// H-ORC-SOFT: soft `quote` reports stale without reverting; hard `price`
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
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 60),
    );

    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10_000;
    });

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(status.stale);
    assert!(!status.valid);
    assert_eq!(status.final_wad, WAD);

    // Hard path must fail closed (not return the stale WAD).
    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "hard price must revert on stale feed; got {hard:?}"
    );
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
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert!(status.deviation);

    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "hard dual out-of-band must revert; got {hard:?}"
    );
}

/// H-ORC-STALE-OWNERSHIP, closed: freshness is gated by **both** the source
/// `max_stale_seconds` and the asset-level `max_price_stale_seconds`.
///
/// v1 keyed multi-feed freshness on the source window alone, so a short asset
/// ceiling was silently overridden by a long source window — an operator who
/// tightened only the asset field got none of the protection they asked for.
/// Two things changed: the read path now dates every leg against the ceiling as
/// well, and `validate_staleness_envelope` rejects a config whose ceiling does
/// not cover its loosest leg, so this shape can no longer be configured at all.
/// Seeded directly here to pin the read-path rule regardless.
#[test]
fn audit_multi_feed_stale_gated_by_both_source_and_asset_windows() {
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
    cfg.sources
        .set(0, redstone_feed(&env, &feed, "BTC/USD", 900));
    client.seed_oracle(&PriceKey::Token(asset.clone()), &cfg);

    // Age 100s: past the 30s asset ceiling, inside the 900s source window.
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 100;
    });
    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "the asset ceiling must gate a leg its own window would have accepted; got {hard:?}"
    );

    // Inside both windows, the same observation reads fine — the ceiling is an
    // additional gate, not a replacement for the per-source window.
    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10;
    });
    assert_eq!(client.price(&PriceKey::Token(asset.clone())).price_wad, WAD);
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
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );
    // Overlap old band but exclude live WAD entirely above the print.
    // Containment probe must fail closed before storage write.
    let result = client.try_set_sanity_band(
        &PriceKey::Token(asset.clone()),
        &(WAD + WAD / 100),
        &(WAD + WAD / 20),
    );
    assert!(
        result.is_err(),
        "set_sanity_band must reject a band that excludes live price; got {result:?}"
    );
    // Config must remain the pre-call band (no partial write).
    let cfg = client.oracle(&PriceKey::Token(asset.clone())).unwrap();
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
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let hard = client.price(&PriceKey::Token(asset.clone()));
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
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(hard.is_err(), "zero price must fail closed; got {hard:?}");
}

/// H-ORC-CFG-NO-PROBE, closed: `set_oracle` now resolves the config
/// before storing it, so a feed that cannot be read is a rejection rather than
/// a stored config that fails closed on every subsequent `price`.
///
/// The v1 behaviour this replaces was not merely an ops annoyance: a market
/// listed against an unreadable feed is a market whose borrows, withdrawals and
/// liquidations all revert, recoverable only by another governance round.
#[test]
fn set_oracle_rejects_a_config_whose_feed_cannot_be_read() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, _) = register_feed(&env);

    // Feed never set, so the containment probe has nothing to resolve.
    let stored = client.try_set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "MISSING", 900),
    );
    assert!(
        stored.is_err(),
        "unreadable feed must not store; got {stored:?}"
    );
    assert!(
        client.oracle(&PriceKey::Token(asset.clone())).is_none(),
        "a rejected config must leave no entry behind"
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
    client.seed_oracle(&PriceKey::Token(asset.clone()), &cfg);

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, WAD);

    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "hard price must revert outside sanity band; got {hard:?}"
    );
}
