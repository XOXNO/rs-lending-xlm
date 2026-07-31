use super::*;
use crate::admin;
use crate::session::Session;
use crate::test_support::{
    in_contract, register_redstone_feed, CountingReflector, LongHistoryReflector,
    TightWindowReflector, TwapReflector, TWAP_OLDER_AGE_SECS,
};
use common::constants::WAD;
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, LpShareSource, MultiFeedRef,
    OracleAssetRef, OracleReadMode, OracleTolerance, PoolKind, ProviderKind, ProviderRef,
    ReflectorFeedRef, ScaledSource,
};
use mock_redstone::MockRedStonePriceFeedClient;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String, Symbol, Vec};

const NOW: u64 = 1_000_000;
const ASSET_CEILING: u64 = 3_600;
const RATIO_BOUND: u64 = 86_400;

fn at_now(env: &Env) {
    env.ledger().set_timestamp(NOW);
}

fn multi_feed(env: &Env, adapter: &Address, feed: &str, max_stale: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed),
            kind: ProviderKind::RedStone,
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }
}

fn oracle(
    env: &Env,
    sources: Vec<PriceSource>,
    ceiling: u64,
    min_wad: i128,
    max_wad: i128,
) -> AssetOracle {
    let _ = env;
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: ceiling,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: min_wad,
        max_sanity_price_wad: max_wad,
    }
}

fn sources(env: &Env, items: &[PriceSource]) -> Vec<PriceSource> {
    let mut out = Vec::new(env);
    for item in items {
        out.push_back(item.clone());
    }
    out
}

fn publish(client: &MockRedStonePriceFeedClient, env: &Env, feed_id: &str, price: i128, age: u64) {
    let ts_ms = (NOW - age) * 1_000;
    client.set_price_data(&String::from_str(env, feed_id), &price, &ts_ms, &ts_ms);
}

fn single_feed_key(env: &Env, adapter: &Address, feed: &str, ceiling: u64) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    admin::store_oracle(
        env,
        &key,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Feed(multi_feed(env, adapter, feed, ceiling))],
            ),
            ceiling,
            WAD / 2,
            2 * WAD,
        ),
    );
    key
}

#[test]
fn test_single_feed_source_resolves() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let feed = resolve(&mut cache, &key, 0);
        assert_eq!(feed.price_wad, WAD);
        assert_eq!(feed.asset_decimals, 8);
    });
}

#[test]
#[should_panic]
fn test_a_feed_past_its_own_bound_reverts() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", WAD, ASSET_CEILING + 1);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

#[test]
#[should_panic]
fn test_a_price_outside_the_sanity_band_reverts() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", 5 * WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

fn dual_key(env: &Env, adapter: &Address, reversed: bool) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    let a = PriceSource::Feed(multi_feed(env, adapter, "A", ASSET_CEILING));
    let b = PriceSource::Feed(multi_feed(env, adapter, "B", ASSET_CEILING));
    let ordered = if reversed { [b, a] } else { [a, b] };
    admin::store_oracle(
        env,
        &key,
        &oracle(env, sources(env, &ordered), ASSET_CEILING, WAD / 2, 2 * WAD),
    );
    key
}

#[test]
fn test_two_agreeing_sources_yield_their_midpoint() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 100, 0);

    in_contract(&env, || {
        let key = dual_key(&env, &adapter, false);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &key, 0).price_wad, WAD + WAD / 200);
    });
}

#[test]
fn test_source_order_changes_neither_price_nor_outcome() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 100, 0);

    let (forward, backward) = in_contract(&env, || {
        let forward_key = dual_key(&env, &adapter, false);
        let backward_key = dual_key(&env, &adapter, true);
        let mut cache = Session::new(&env);
        (
            resolve(&mut cache, &forward_key, 0).price_wad,
            resolve(&mut cache, &backward_key, 0).price_wad,
        )
    });
    assert_eq!(forward, backward);
}

#[test]
#[should_panic]
fn test_two_disagreeing_sources_revert() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD + WAD / 2, 0);

    in_contract(&env, || {
        let key = dual_key(&env, &adapter, false);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

fn scaled_setup(
    env: &Env,
    adapter: &Address,
    ceiling: u64,
    min_factor: i128,
    max_factor: i128,
) -> PriceKey {
    let btc = PriceKey::Ref(Symbol::new(env, "BTC"));
    admin::store_oracle(
        env,
        &btc,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Feed(multi_feed(env, adapter, "BTC", 3_600))],
            ),
            3_600,
            WAD / 2,
            1_000_000 * WAD,
        ),
    );

    let token = PriceKey::Token(Address::generate(env));
    admin::store_oracle(
        env,
        &token,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Scaled(ScaledSource {
                    factor: multi_feed(env, adapter, "RATIO", RATIO_BOUND),
                    quote: btc,
                    min_factor_wad: min_factor,
                    max_factor_wad: max_factor,
                })],
            ),
            ceiling,
            WAD / 2,
            1_000_000 * WAD,
        ),
    );
    token
}

#[test]
fn test_scaled_source_multiplies_ratio_by_quote() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD + WAD / 100, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut cache = Session::new(&env);

        assert_eq!(resolve(&mut cache, &token, 0).price_wad, 101 * WAD);
    });
}

#[test]
fn test_shared_nested_quote_is_composed_once_per_session() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "RATIO_A", WAD, 0);
    publish(&client, &env, "RATIO_B", WAD, 0);
    let reflector = env.register(CountingReflector, ());

    in_contract(&env, || {
        let quote = PriceKey::Ref(Symbol::new(&env, "QUOTE"));
        admin::store_oracle(
            &env,
            &quote,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Feed(FeedSource {
                        provider: ProviderRef::Reflector(ReflectorFeedRef {
                            contract: reflector.clone(),
                            asset: OracleAssetRef::Symbol(Symbol::new(&env, "QUOTE")),
                            read_mode: OracleReadMode::Spot,
                        }),
                        decimals: 14,
                        max_stale_seconds: ASSET_CEILING,
                    })],
                ),
                ASSET_CEILING,
                1,
                10 * WAD,
            ),
        );

        let parent = |feed_id: &str| {
            let key = PriceKey::Token(Address::generate(&env));
            admin::store_oracle(
                &env,
                &key,
                &oracle(
                    &env,
                    sources(
                        &env,
                        &[PriceSource::Scaled(ScaledSource {
                            factor: multi_feed(&env, &adapter, feed_id, ASSET_CEILING),
                            quote: quote.clone(),
                            min_factor_wad: WAD,
                            max_factor_wad: WAD,
                        })],
                    ),
                    ASSET_CEILING,
                    1,
                    10 * WAD,
                ),
            );
            key
        };
        let first = parent("RATIO_A");
        let second = parent("RATIO_B");
        let mut session = Session::new(&env);

        let first_price = resolve(&mut session, &first, 0).price_wad;
        let second_price = resolve(&mut session, &second, 0).price_wad;
        assert_eq!(first_price, second_price);
    });
}

/// Failure-path twin of the test above: a quote that fails is memoized as an
/// error, so a second parent sharing that quote must reuse the verdict instead
/// of re-reading the provider.
///
/// The assertion is behavioural rather than a read counter. `CountingReflector`
/// returns `reads * 1 WAD`, so read 1 (1 WAD) sits below the quote's sanity
/// floor while read 2 (2 WAD) sits inside the band. Recomputing therefore does
/// not merely cost a call — it *changes the verdict*, and the second parent
/// resolves successfully. That is what lets this test kill a no-op
/// `Session::store_error` and a `Session::cached_error` stubbed to `None`,
/// which are otherwise equivalent mutants: on a deterministic provider,
/// recomputation returns the same error and nothing observable differs.
#[test]
fn test_failed_nested_quote_is_memoized_per_session() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "RATIO_A", WAD, 0);
    publish(&client, &env, "RATIO_B", WAD, 0);
    let reflector = env.register(CountingReflector, ());

    in_contract(&env, || {
        let quote = PriceKey::Ref(Symbol::new(&env, "QUOTE"));
        admin::store_oracle(
            &env,
            &quote,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Feed(FeedSource {
                        provider: ProviderRef::Reflector(ReflectorFeedRef {
                            contract: reflector.clone(),
                            asset: OracleAssetRef::Symbol(Symbol::new(&env, "QUOTE")),
                            read_mode: OracleReadMode::Spot,
                        }),
                        decimals: 14,
                        max_stale_seconds: ASSET_CEILING,
                    })],
                ),
                ASSET_CEILING,
                // Straddles the two reads: 1 WAD is out of band, 2 WAD is in.
                3 * WAD / 2,
                10 * WAD,
            ),
        );

        let parent = |feed_id: &str| {
            let key = PriceKey::Token(Address::generate(&env));
            admin::store_oracle(
                &env,
                &key,
                &oracle(
                    &env,
                    sources(
                        &env,
                        &[PriceSource::Scaled(ScaledSource {
                            factor: multi_feed(&env, &adapter, feed_id, ASSET_CEILING),
                            quote: quote.clone(),
                            min_factor_wad: WAD,
                            max_factor_wad: WAD,
                        })],
                    ),
                    ASSET_CEILING,
                    1,
                    10 * WAD,
                ),
            );
            key
        };
        let first = parent("RATIO_A");
        let second = parent("RATIO_B");
        let mut session = Session::new(&env);

        assert!(matches!(
            resolve_nested(&mut session, &first, 0),
            Err(common::errors::OracleError::SanityBoundViolated)
        ));
        // Served from the error memo. Without it the quote is read a second
        // time, clears the band, and this resolves to Ok(2 WAD).
        assert!(matches!(
            resolve_nested(&mut session, &second, 0),
            Err(common::errors::OracleError::SanityBoundViolated)
        ));
    });
}

#[test]
fn test_cached_nested_quote_still_enforces_depth_backstop() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut session = Session::new(&env);
        assert!(resolve_nested(&mut session, &token, 0).is_ok());
        assert!(matches!(
            resolve_nested(&mut session, &token, MAX_RESOLUTION_DEPTH),
            Err(common::errors::OracleError::OracleDepthExceeded)
        ));
    });
}

#[test]
#[should_panic]
fn test_a_ratio_outside_its_bounds_reverts() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD / 2, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #217)")]
fn test_scaled_product_overflow_is_typed_invalid_price_not_host_trap() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);

    let huge_raw = i128::MAX / 10i128.pow(10);
    publish(&client, &env, "BTC", huge_raw, 0);
    publish(&client, &env, "RATIO", huge_raw, 0);

    in_contract(&env, || {
        let btc = PriceKey::Ref(Symbol::new(&env, "BTC"));
        admin::store_oracle(
            &env,
            &btc,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Feed(multi_feed(&env, &adapter, "BTC", 3_600))],
                ),
                3_600,
                1,
                i128::MAX,
            ),
        );
        let token = PriceKey::Token(Address::generate(&env));
        admin::store_oracle(
            &env,
            &token,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, "RATIO", RATIO_BOUND),
                        quote: btc,
                        min_factor_wad: 1,
                        max_factor_wad: i128::MAX,
                    })],
                ),
                RATIO_BOUND,
                1,
                i128::MAX,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
#[should_panic]
fn test_a_frozen_slow_leg_cannot_ride_under_a_live_fast_one() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, ASSET_CEILING * 2);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, ASSET_CEILING, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &token, 0);
    });
}

#[test]
fn test_the_same_frozen_leg_prices_under_a_ceiling_that_allows_it() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, ASSET_CEILING * 2);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &token, 0).price_wad, 100 * WAD);
    });
}

#[test]
fn test_a_composite_reports_the_freshness_of_its_weaker_leg() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);
    publish(&client, &env, "RATIO", WAD, 600);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD / 2, 2 * WAD);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &token, 0).timestamp, NOW - 600);
    });
}

#[test]
#[should_panic]
fn test_an_unconfigured_key_reverts_rather_than_pricing_zero() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &PriceKey::Ref(Symbol::new(&env, "NOPE")), 0);
    });
}

#[test]
#[should_panic]
fn test_lp_shares_are_not_priceable_yet() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        admin::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::LpShare(LpShareSource {
                        pool: Address::generate(&env),
                        kind: PoolKind::ConstantProduct,
                        key_a: PriceKey::Ref(Symbol::new(&env, "A")),
                        key_b: PriceKey::Ref(Symbol::new(&env, "B")),
                        reserve_a_decimals: 7,
                        reserve_b_decimals: 7,
                        share_decimals: 7,
                    })],
                ),
                ASSET_CEILING,
                1,
                i128::MAX / 2,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

#[test]
#[should_panic]
fn test_a_scaled_cycle_reverts_at_read_time_too() {
    let env = Env::default();
    at_now(&env);
    let (adapter, _client) = register_redstone_feed(&env);

    in_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "LOOP"));
        admin::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Scaled(ScaledSource {
                        factor: multi_feed(&env, &adapter, "RATIO", RATIO_BOUND),
                        quote: key.clone(),
                        min_factor_wad: 1,
                        max_factor_wad: i128::MAX,
                    })],
                ),
                ASSET_CEILING,
                1,
                i128::MAX / 2,
            ),
        );
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
}

fn resolve_twap(env: &Env, contract: &Address, records: u32) -> PriceFeedRaw {
    let key = PriceKey::Token(Address::generate(env));
    admin::store_oracle(
        env,
        &key,
        &oracle(
            env,
            sources(
                env,
                &[PriceSource::Feed(FeedSource {
                    provider: ProviderRef::Reflector(ReflectorFeedRef {
                        contract: contract.clone(),
                        asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
                        read_mode: OracleReadMode::Twap(records),
                    }),
                    decimals: 14,
                    max_stale_seconds: ASSET_CEILING,
                })],
            ),
            ASSET_CEILING,
            1,
            10 * WAD,
        ),
    );
    let mut session = Session::new(env);
    resolve(&mut session, &key, 0)
}

#[test]
fn test_twap_read_averages_the_window_and_dates_itself_to_the_oldest_sample() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(TwapReflector, ());

    let feed = in_contract(&env, || resolve_twap(&env, &reflector, 2));

    assert_eq!(feed.price_wad, 2 * WAD);

    assert_eq!(feed.timestamp, NOW - TWAP_OLDER_AGE_SECS);
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_twap_read_rejects_a_history_shorter_than_the_window_needs() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(TwapReflector, ());
    in_contract(&env, || {
        resolve_twap(&env, &reflector, 6);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_twap_read_rejects_more_samples_than_it_asked_for() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(LongHistoryReflector, ());
    in_contract(&env, || {
        resolve_twap(&env, &reflector, 2);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_twap_read_rejects_samples_spaced_tighter_than_the_resolution() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(TightWindowReflector, ());
    in_contract(&env, || {
        resolve_twap(&env, &reflector, 2);
    });
}

#[test]
fn test_the_depth_backstop_admits_the_cap_and_rejects_everything_past_it() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "USST", WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "USST", ASSET_CEILING);
        let mut session = Session::new(&env);

        assert!(resolve_nested(&mut session, &key, MAX_RESOLUTION_DEPTH).is_ok());
        assert!(matches!(
            resolve_nested(&mut session, &key, MAX_RESOLUTION_DEPTH + 2),
            Err(common::errors::OracleError::OracleDepthExceeded)
        ));

        assert!(resolve_nested(&mut session, &key, MAX_RESOLUTION_DEPTH).is_ok());
        assert!(matches!(
            resolve_nested(&mut session, &key, MAX_RESOLUTION_DEPTH + 2),
            Err(common::errors::OracleError::OracleDepthExceeded)
        ));
    });
}

#[test]
fn test_a_scaled_chain_past_the_cap_is_rejected_by_the_depth_it_accumulates() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BASE", WAD, 0);
    publish(&client, &env, "RATIO", WAD, 0);

    in_contract(&env, || {
        let mut current = PriceKey::Ref(Symbol::new(&env, "L0"));
        admin::store_oracle(
            &env,
            &current,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::Feed(multi_feed(
                        &env,
                        &adapter,
                        "BASE",
                        RATIO_BOUND,
                    ))],
                ),
                RATIO_BOUND,
                WAD / 2,
                2 * WAD,
            ),
        );

        let level_names = ["L1", "L2", "L3", "L4", "L5"];
        assert!(
            level_names.len() as u32 > MAX_RESOLUTION_DEPTH + 1,
            "the chain has to reach past the cap for this test to mean anything"
        );
        for name in level_names {
            let key = PriceKey::Ref(Symbol::new(&env, name));
            admin::store_oracle(
                &env,
                &key,
                &oracle(
                    &env,
                    sources(
                        &env,
                        &[PriceSource::Scaled(ScaledSource {
                            factor: multi_feed(&env, &adapter, "RATIO", RATIO_BOUND),
                            quote: current.clone(),
                            min_factor_wad: WAD,
                            max_factor_wad: WAD,
                        })],
                    ),
                    RATIO_BOUND,
                    WAD / 2,
                    2 * WAD,
                ),
            );
            current = key;
        }

        let mut session = Session::new(&env);
        assert!(matches!(
            resolve_nested(&mut session, &current, 0),
            Err(common::errors::OracleError::OracleDepthExceeded)
        ));
    });
}
