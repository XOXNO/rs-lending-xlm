use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance,
    PriceKey, PriceSource, ProviderKind, ProviderRef, ScaledSource, TrustDomain,
};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use price_aggregator::{Error, PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::storage::Instance as _;
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

#[test]
fn public_entrypoint_renews_instance_ttl() {
    let env = Env::default();
    let (_owner, client) = register_agg(&env);
    let initial = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(initial, TTL_BUMP_INSTANCE);

    env.ledger().with_mut(|li| {
        li.sequence_number += TTL_BUMP_INSTANCE - TTL_THRESHOLD_INSTANCE + 1;
    });
    let aged = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert!(aged < TTL_THRESHOLD_INSTANCE);

    let _ = client.get_owner();

    let renewed = env.as_contract(&client.address, || env.storage().instance().get_ttl());
    assert_eq!(renewed, TTL_BUMP_INSTANCE);
}

fn redstone_single(env: &Env, feed: &Address, feed_id: &str, max_stale: u64) -> AssetOracle {
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: max_stale,
        sources: soroban_sdk::vec![env, redstone_feed(env, feed, feed_id, max_stale)],

        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,

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

    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
    let cfg = redstone_single(&env, &feed, "BTC/USD", 900);

    client.set_oracle(&PriceKey::Token(asset.clone()), &cfg);

    assert_eq!(env.events().all().events().len(), 1);
    assert_eq!(client.oracle(&PriceKey::Token(asset.clone())), Some(cfg));
}

#[test]
fn child_reconfiguration_cannot_invalidate_parent_independence() {
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let (direct_adapter, direct) = register_feed(&env);
    let (factor_adapter, factor) = register_feed(&env);
    let (quote_adapter, quote) = register_feed(&env);
    direct.set_price(&String::from_str(&env, "DIRECT"), &WAD);
    factor.set_price(&String::from_str(&env, "FACTOR"), &WAD);
    quote.set_price(&String::from_str(&env, "QUOTE"), &WAD);

    let quote_key = PriceKey::Ref(soroban_sdk::Symbol::new(&env, "QUOTE"));
    let mut original_quote = redstone_single(&env, &quote_adapter, "QUOTE", 900);
    original_quote.asset_decimals = 0;
    client.set_oracle(&quote_key, &original_quote);

    let factor_feed = match redstone_feed(&env, &factor_adapter, "FACTOR", 900) {
        PriceSource::Feed(feed) => feed,
        _ => unreachable!(),
    };
    let parent_key = PriceKey::Token(Address::generate(&env));
    let parent = AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        sources: soroban_sdk::vec![
            &env,
            redstone_feed(&env, &direct_adapter, "DIRECT", 900),
            PriceSource::Scaled(ScaledSource {
                factor: factor_feed,
                quote: quote_key.clone(),
                min_factor_wad: WAD,
                max_factor_wad: WAD,
            }),
        ],
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: WAD / 2,
        max_sanity_price_wad: 2 * WAD,
    };
    client.set_oracle(&parent_key, &parent);

    let mut repointed = redstone_single(&env, &direct_adapter, "DIRECT", 900);
    repointed.asset_decimals = 0;
    assert!(client.try_set_oracle(&quote_key, &repointed).is_err());
    assert_eq!(client.oracle(&quote_key), Some(original_quote));
    assert_eq!(client.price(&parent_key).price_wad, WAD);
}

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
    assert_eq!(
        bulk.get(PriceKey::Token(asset.clone())).unwrap().price_wad,
        WAD
    );
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
    let env = Env::default();
    env.mock_all_auths();
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);
}

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
    feed_client.set_price(&feed_id, &0);
    client.seed_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let status = client.quote(&PriceKey::Token(asset.clone()));
    assert!(!status.valid);
    assert_eq!(status.final_wad, 0);

    assert!(client.try_price(&PriceKey::Token(asset.clone())).is_err());
}

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
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD + WAD / 100));
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

    client.set_sanity_band(
        &PriceKey::Token(asset.clone()),
        &(WAD - WAD / 20),
        &(WAD + WAD / 20),
    );
    let after_band = client.oracle(&PriceKey::Token(asset.clone())).unwrap();
    assert_eq!(after_band.min_sanity_price_wad, WAD - WAD / 20);
    assert_eq!(after_band.max_sanity_price_wad, WAD + WAD / 20);

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
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);

    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD + WAD / 10));

    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 12_500, 8_000),
    );

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
fn constructor_sets_owner_and_ownership_is_not_exported() {
    let env = Env::default();
    env.mock_all_auths();
    let (owner, client) = register_agg(&env);
    assert_eq!(client.get_owner(), Some(owner));
}

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

    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "hard price must revert on stale feed; got {hard:?}"
    );
}

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

    let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
    cfg.max_price_stale_seconds = 30;
    cfg.sources
        .set(0, redstone_feed(&env, &feed, "BTC/USD", 900));
    client.seed_oracle(&PriceKey::Token(asset.clone()), &cfg);

    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 100;
    });
    let hard = client.try_price(&PriceKey::Token(asset.clone()));
    assert!(
        hard.is_err(),
        "the asset ceiling must gate a leg its own window would have accepted; got {hard:?}"
    );

    env.ledger().with_mut(|li| {
        li.timestamp = 1_000 + 10;
    });
    assert_eq!(client.price(&PriceKey::Token(asset.clone())).price_wad, WAD);
}

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

    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_single(&env, &feed, "BTC/USD", 900),
    );

    let result = client.try_set_sanity_band(
        &PriceKey::Token(asset.clone()),
        &(WAD + WAD / 100),
        &(WAD + WAD / 20),
    );
    assert!(
        result.is_err(),
        "set_sanity_band must reject a band that excludes live price; got {result:?}"
    );

    let cfg = client.oracle(&PriceKey::Token(asset.clone())).unwrap();
    assert_eq!(cfg.min_sanity_price_wad, WAD - WAD / 20);
    assert_eq!(cfg.max_sanity_price_wad, WAD + WAD / 20);
}

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
    let anchor = WAD + WAD / 50;
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &primary);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &anchor);
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let hard = client.price(&PriceKey::Token(asset.clone()));
    assert_eq!(hard.price_wad, (primary + anchor) / 2);
}

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

#[test]
fn price_spread_of_a_single_source_is_the_degenerate_interval() {
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

    assert_eq!(
        client.price_spread(&PriceKey::Token(asset.clone())),
        (WAD, WAD)
    );
    assert_eq!(client.price(&PriceKey::Token(asset.clone())).price_wad, WAD);
}

#[test]
fn price_spread_of_dual_legs_brackets_the_reported_price() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let anchor = WAD + WAD / 100;
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &anchor);
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    let (low, high) = client.price_spread(&PriceKey::Token(asset.clone()));
    assert_eq!((low, high), (WAD, anchor));
    assert!(low < high, "two disagreeing legs span a real interval");

    let price = client.price(&PriceKey::Token(asset.clone())).price_wad;
    assert_eq!(price, (WAD + anchor) / 2);
    assert!(low <= price && price <= high);
}

#[test]
fn price_spread_orders_its_interval_regardless_of_leg_order() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });
    let (_owner, client) = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_feed(&env);
    let anchor = WAD + WAD / 100;
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &anchor);
    feed_client.set_price(&String::from_str(&env, "ANCHOR"), &WAD);
    client.set_oracle(
        &PriceKey::Token(asset.clone()),
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_524),
    );

    assert_eq!(
        client.price_spread(&PriceKey::Token(asset.clone())),
        (WAD, anchor)
    );
}

#[test]
fn hard_price_accepts_the_inclusive_edges_of_the_sanity_band() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.timestamp = 1_700_000_000;
    });

    for (min, max) in [(WAD, WAD + WAD / 10), (WAD - WAD / 20, WAD)] {
        let (_owner, client) = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
        cfg.min_sanity_price_wad = min;
        cfg.max_sanity_price_wad = max;
        client.set_oracle(&PriceKey::Token(asset.clone()), &cfg);

        assert_eq!(
            client.price(&PriceKey::Token(asset.clone())).price_wad,
            WAD,
            "price on the band edge [{min}, {max}] must resolve"
        );
    }
}
