use super::*;
use crate::registry;
use crate::session::Session;
use crate::test_support::{
    in_contract, register_redstone_feed, CountingReflector, LongHistoryReflector,
    TightWindowReflector, TwapReflector, TWAP_OLDER_AGE_SECS,
};
use common::constants::WAD;
use common::oracle::observation::MAX_LEG_AGE_SPREAD_SECONDS;
use common::types::{
    AquariusLpSource, AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef,
    OracleAssetRef, OracleReadMode, OracleTolerance, ProviderRef, ReflectorFeedRef, ScaledSource,
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
    nature_feed(env, adapter, feed, max_stale, FeedNature::Fundamental)
}

fn nature_feed(
    env: &Env,
    adapter: &Address,
    feed: &str,
    max_stale: u64,
    nature: FeedNature,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, feed),
            nature,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }
}

/// Mainnet's shape: a fast market leg blended with a slow fundamental leg whose
/// own bound legitimately allows it to lag by hours.
const SLOW_LEG_BOUND: u64 = 57_600;

fn mixed_pair_key(env: &Env, adapter: &Address, fast: FeedNature, slow: FeedNature) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    let a = PriceSource::Feed(nature_feed(env, adapter, "A", ASSET_CEILING, fast));
    let b = PriceSource::Feed(nature_feed(env, adapter, "B", SLOW_LEG_BOUND, slow));
    registry::store_oracle(
        env,
        &key,
        &oracle(env, sources(env, &[a, b]), SLOW_LEG_BOUND, WAD / 2, 2 * WAD),
    );
    key
}

#[test]
fn test_a_fundamental_leg_may_lag_its_market_partner_past_the_spread_bound() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    // Five hours old: inside its own 57600s bound, far outside the spread bound.
    publish(&client, &env, "B", WAD, 18_000);

    in_contract(&env, || {
        let key = mixed_pair_key(&env, &adapter, FeedNature::Market, FeedNature::Fundamental);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &key, 0).price_wad, WAD);
    });
}

/// The bound is exclusive: legs exactly `MAX_LEG_AGE_SPREAD_SECONDS` apart are
/// still blended. Kills the `>` -> `>=` mutant at the comparison.
#[test]
fn test_two_market_legs_exactly_at_the_spread_bound_still_price() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD, MAX_LEG_AGE_SPREAD_SECONDS);

    in_contract(&env, || {
        let key = mixed_pair_key(&env, &adapter, FeedNature::Market, FeedNature::Market);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &key, 0).price_wad, WAD);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #206)")]
fn test_two_market_legs_still_cannot_straddle_the_spread_bound() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "A", WAD, 0);
    publish(&client, &env, "B", WAD, 18_000);

    in_contract(&env, || {
        let key = mixed_pair_key(&env, &adapter, FeedNature::Market, FeedNature::Market);
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &key, 0);
    });
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
    registry::store_oracle(
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
#[should_panic(expected = "Error(Contract, #206)")]
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
#[should_panic(expected = "Error(Contract, #223)")]
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
    registry::store_oracle(
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
#[should_panic(expected = "Error(Contract, #205)")]
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
    registry::store_oracle(
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
    registry::store_oracle(
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
        registry::store_oracle(
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
            registry::store_oracle(
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
        registry::store_oracle(
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
                3 * WAD / 2,
                10 * WAD,
            ),
        );

        let parent = |feed_id: &str| {
            let key = PriceKey::Token(Address::generate(&env));
            registry::store_oracle(
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
#[should_panic(expected = "Error(Contract, #230)")]
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
        registry::store_oracle(
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
        registry::store_oracle(
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
#[should_panic(expected = "Error(Contract, #206)")]
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
#[should_panic(expected = "Error(Contract, #216)")]
fn test_an_unconfigured_key_reverts_rather_than_pricing_zero() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let mut cache = Session::new(&env);
        let _ = resolve(&mut cache, &PriceKey::Ref(Symbol::new(&env, "NOPE")), 0);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_lp_reverts_when_underlyings_missing() {
    let env = Env::default();
    at_now(&env);
    in_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        registry::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::AquariusLp(AquariusLpSource {
                        pool: Address::generate(&env),
                        token_a: Address::generate(&env),
                        token_b: Address::generate(&env),
                        key_a: PriceKey::Ref(Symbol::new(&env, "A")),
                        key_b: PriceKey::Ref(Symbol::new(&env, "B")),
                        reserve_a_decimals: 7,
                        reserve_b_decimals: 7,
                        min_pool_value_wad: 1,
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
#[should_panic(expected = "Error(Contract, #210)")]
fn test_a_scaled_cycle_reverts_at_read_time_too() {
    let env = Env::default();
    at_now(&env);
    let (adapter, _client) = register_redstone_feed(&env);

    in_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "LOOP"));
        registry::store_oracle(
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
    registry::store_oracle(
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
fn test_twap_read_accepts_the_current_period_plus_the_window() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(crate::test_support::PlusOneReflector, ());
    in_contract(&env, || {
        assert_eq!(resolve_twap(&env, &reflector, 2).price_wad, WAD);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_twap_read_rejects_more_samples_than_the_window_plus_current() {
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
fn test_cached_lp_price_still_checks_both_dependency_depths() {
    let env = Env::default();
    at_now(&env);

    in_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        registry::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(
                    &env,
                    &[PriceSource::AquariusLp(AquariusLpSource {
                        pool: Address::generate(&env),
                        token_a: Address::generate(&env),
                        token_b: Address::generate(&env),
                        key_a: PriceKey::Ref(Symbol::new(&env, "A")),
                        key_b: PriceKey::Ref(Symbol::new(&env, "B")),
                        reserve_a_decimals: 7,
                        reserve_b_decimals: 7,
                        min_pool_value_wad: 1,
                    })],
                ),
                ASSET_CEILING,
                1,
                2 * WAD,
            ),
        );
        let mut session = Session::new(&env);
        session.store_price(
            &key,
            PriceFeedRaw {
                price_wad: WAD,
                asset_decimals: 7,
                timestamp: NOW,
            },
        );

        assert!(matches!(
            resolve_nested(&mut session, &key, MAX_RESOLUTION_DEPTH),
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
        registry::store_oracle(
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
            registry::store_oracle(
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

#[test]
fn test_a_scaled_factor_outside_its_band_is_rejected() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 50_000 * WAD, 0);
    publish(&client, &env, "RATIO", 3 * WAD, 0);

    in_contract(&env, || {
        let token = scaled_setup(&env, &adapter, RATIO_BOUND, WAD, 2 * WAD);
        let mut cache = Session::new(&env);
        assert!(!resolve_status(&mut cache, &token, 0).valid);
    });
}

/// `to_status` is what `PriceAggregator::quotes` returns, and it had no test —
/// found by sweeping the 84 common/price-aggregator Certora rules for
/// production functions with no Rust coverage. Two rules reference it; nothing
/// else did.
///
/// The security-relevant half is the first branch: an outcome carrying an error
/// must collapse to `unusable()` and nothing else. A consumer reads `valid` to
/// decide whether to act on a price, so an errored resolution leaking a
/// non-zero price with `valid: true` is the failure that matters.
#[test]
fn to_status_collapses_an_errored_outcome_to_unusable() {
    let mut outcome = Outcome::blank();
    // Populate every field, so a wrong branch would visibly leak one of them.
    outcome.price_wad = 123 * WAD;
    outcome.first_wad = 111 * WAD;
    outcome.second_wad = 222 * WAD;
    outcome.timestamp = 1_700_000_000;
    outcome.stale = true;
    outcome.deviation = true;
    outcome.err = Some(OracleError::NoLastPrice);

    let status = to_status(&outcome, None);
    assert_eq!(
        status,
        PriceStatus::unusable(),
        "errored outcome must not leak any field"
    );
    assert!(!status.valid, "errored outcome reported valid");
    assert_eq!(status.final_wad, 0);
    assert_eq!(status.primary_wad, 0);
    assert_eq!(status.secondary_wad, 0);
}

/// The other branch: a clean outcome must map straight through, each field to
/// its counterpart. Primary and secondary are distinct values here so a swapped
/// mapping cannot pass — that is the mistake this shape is chosen to catch.
#[test]
fn to_status_maps_a_clean_outcome_field_for_field() {
    let mut outcome = Outcome::blank();
    outcome.price_wad = 150 * WAD;
    outcome.first_wad = 149 * WAD;
    outcome.second_wad = 151 * WAD;
    outcome.timestamp = 1_700_000_500;
    outcome.stale = false;
    outcome.deviation = false;
    outcome.err = None;

    let status = to_status(&outcome, None);
    assert_eq!(status.final_wad, 150 * WAD);
    assert_eq!(
        status.primary_wad,
        149 * WAD,
        "primary/secondary mapped the wrong way round"
    );
    assert_eq!(
        status.secondary_wad,
        151 * WAD,
        "primary/secondary mapped the wrong way round"
    );
    assert_eq!(status.price_timestamp, 1_700_000_500);
    assert!(!status.stale);
    assert!(!status.deviation);
}

/// Flags must survive the mapping even when the outcome carries no error: a
/// stale or deviating reading is still reported, and it is the consumer's job
/// to weigh them. Silently clearing either would hide exactly the condition
/// they exist to signal.
#[test]
fn to_status_preserves_stale_and_deviation_flags_without_an_error() {
    let mut outcome = Outcome::blank();
    outcome.price_wad = WAD;
    outcome.timestamp = 42;
    outcome.stale = true;
    outcome.deviation = true;
    outcome.err = None;

    let status = to_status(&outcome, None);
    assert!(status.stale, "stale flag dropped");
    assert!(status.deviation, "deviation flag dropped");
    assert_eq!(status.final_wad, WAD);
}
