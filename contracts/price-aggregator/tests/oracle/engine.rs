use super::*;
use crate::registry;
use crate::session::Session;
use crate::test_support::{
    in_contract, register_redstone_feed, sample, sample_run, CountingReflector, GappedReflector,
    LongHistoryReflector, ScriptedReflector, ScriptedReflectorClient, TightWindowReflector,
    TwapReflector, REFLECTOR_RESOLUTION_SECS, TWAP_GAPPED_OLDER_AGE_SECS, TWAP_OLDER_AGE_SECS,
};
use common::constants::WAD;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_LEG_AGE_SPREAD_SECONDS};
use common::oracle::providers::reflector::ReflectorPriceData;
use common::types::{
    AquariusLpSource, AssetOracle, ChainlinkFeedRef, FeedNature, FeedSource, IndependencePolicy,
    MultiFeedRef, OracleAssetRef, OracleReadMode, OracleTolerance, ProviderRef, ReflectorFeedRef,
    ScaledSource,
};
use mock_redstone::MockRedStonePriceFeedClient;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, BytesN, String, Symbol, Vec};

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

fn chainlink_feed(env: &Env, proxy: &Address, max_stale: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Chainlink(ChainlinkFeedRef {
            contract: proxy.clone(),
            data_id: BytesN::from_array(env, &[9; 32]),
        }),
        decimals: 18,
        max_stale_seconds: max_stale,
    }
}

#[test]
fn test_chainlink_market_leg_blends_with_redstone_fundamental_leg() {
    let env = Env::default();
    at_now(&env);
    let (proxy, chainlink) = register_redstone_feed(&env);
    let (redstone, redstone_client) = register_redstone_feed(&env);
    chainlink.set_chainlink_price(&BytesN::from_array(&env, &[9; 32]), &WAD);
    publish(&redstone_client, &env, "USDT0", WAD, 0);

    let key = PriceKey::Token(Address::generate(&env));
    let chainlink = PriceSource::Feed(chainlink_feed(&env, &proxy, 93_600));
    let redstone = PriceSource::Feed(nature_feed(
        &env,
        &redstone,
        "USDT0",
        57_600,
        FeedNature::Fundamental,
    ));
    let price = in_contract(&env, || {
        registry::store_oracle(
            &env,
            &key,
            &oracle(
                &env,
                sources(&env, &[chainlink, redstone]),
                93_600,
                WAD * 95 / 100,
                WAD * 105 / 100,
            ),
        );
        resolve(&mut Session::new(&env), &key, 0)
    });
    assert_eq!(price.price_wad, WAD);
    assert_eq!(price.timestamp, NOW);
}

const CHAINLINK_BOUND: u64 = 93_600;

fn chainlink_status(
    env: &Env,
    proxy: &Address,
    client: &MockRedStonePriceFeedClient,
    timestamp: u64,
) -> PriceStatus {
    client.set_chainlink_round(&BytesN::from_array(env, &[9; 32]), &WAD, &timestamp);
    let key = PriceKey::Token(Address::generate(env));
    let source = PriceSource::Feed(chainlink_feed(env, proxy, CHAINLINK_BOUND));
    in_contract(env, || {
        registry::store_oracle(
            env,
            &key,
            &oracle(
                env,
                sources(env, &[source]),
                CHAINLINK_BOUND,
                WAD * 95 / 100,
                WAD * 105 / 100,
            ),
        );
        resolve_status(&mut Session::new(env), &key, 0)
    })
}

/// M-2: the Chainlink staleness bound is inclusive at the aggregator level.
#[test]
fn chainlink_round_at_the_stale_boundary_is_fresh_and_one_second_older_is_stale() {
    let env = Env::default();
    at_now(&env);
    let (proxy, client) = register_redstone_feed(&env);

    let fresh = chainlink_status(&env, &proxy, &client, NOW - CHAINLINK_BOUND);
    assert!(fresh.valid, "{fresh:?}");
    assert!(!fresh.stale);
    assert_eq!(fresh.final_wad, WAD);

    let stale = chainlink_status(&env, &proxy, &client, NOW - CHAINLINK_BOUND - 1);
    assert!(stale.stale, "{stale:?}");
    assert!(!stale.valid);
}

/// M-2: a Chainlink round dated past the future-skew allowance is unreadable.
#[test]
fn chainlink_round_beyond_the_future_skew_is_rejected() {
    let env = Env::default();
    at_now(&env);
    let (proxy, client) = register_redstone_feed(&env);

    let skewed = chainlink_status(&env, &proxy, &client, NOW + MAX_FUTURE_SKEW_SECONDS);
    assert!(skewed.valid, "{skewed:?}");
    assert_eq!(skewed.final_wad, WAD);

    let future = chainlink_status(&env, &proxy, &client, NOW + MAX_FUTURE_SKEW_SECONDS + 1);
    assert!(!future.valid, "{future:?}");
    assert_eq!(future.final_wad, 0);
}

/// M-2: a zero-timestamp Chainlink round reads as stale, never as a price.
#[test]
fn chainlink_round_with_a_zero_timestamp_is_rejected() {
    let env = Env::default();
    at_now(&env);
    let (proxy, client) = register_redstone_feed(&env);

    let zero = chainlink_status(&env, &proxy, &client, 0);
    assert!(zero.stale, "{zero:?}");
    assert!(!zero.valid);
}

#[test]
fn status_resolution_returns_the_cached_status_without_requerying_an_oracle() {
    let env = Env::default();
    let key = PriceKey::Token(Address::generate(&env));
    in_contract(&env, || {
        let mut session = Session::new(&env);
        let cached = PriceStatus {
            final_wad: 123,
            valid: true,
            ..PriceStatus::unusable()
        };
        session.store_status(&key, cached.clone());

        // No oracle was configured for this key. Recomputing would return an
        // unusable status; the stored result must be reused within this call.
        assert_eq!(resolve_status(&mut session, &key, 0), cached);
    });
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
    // The leg is five hours old. That is inside its own 57600s bound and far outside the
    // spread bound.
    publish(&client, &env, "B", WAD, 18_000);

    in_contract(&env, || {
        let key = mixed_pair_key(&env, &adapter, FeedNature::Market, FeedNature::Fundamental);
        let mut cache = Session::new(&env);
        assert_eq!(resolve(&mut cache, &key, 0).price_wad, WAD);
    });
}

/// The bound is inclusive. Legs exactly `MAX_LEG_AGE_SPREAD_SECONDS` apart are still
/// blended. The test kills the mutant that changes `>` to `>=` at the comparison.
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

// The age-spread check compares the newest sample of each market leg. A
// `Twap(records)` leg enters it with its newest round, not its oldest.

/// Budget, in seconds, of every leg and of the asset in the spread tests.
const SPREAD_TEST_BOUND: u64 = 7_200;

/// A `Twap(records)` leg on a fresh `ScriptedReflector`. The answer holds
/// `records` rounds of 1 dollar, `resolution` seconds apart, the newest
/// `newest_age` seconds old.
fn twap_leg(env: &Env, records: u32, resolution: u32, newest_age: u64) -> PriceSource {
    let reflector = env.register(ScriptedReflector, ());
    let answer = sample_run(env, NOW - newest_age, u64::from(resolution), records, 0);
    ScriptedReflectorClient::new(env, &reflector).script(&resolution, &answer);
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: reflector,
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Twap(records),
        }),
        decimals: 14,
        max_stale_seconds: SPREAD_TEST_BOUND,
    })
}

/// A spot multi-feed leg of `nature` on `feed_id`.
fn spot_leg(env: &Env, adapter: &Address, feed_id: &str, nature: FeedNature) -> PriceSource {
    PriceSource::Feed(nature_feed(
        env,
        adapter,
        feed_id,
        SPREAD_TEST_BOUND,
        nature,
    ))
}

/// Registers the aggregator the spread tests read through.
fn spread_aggregator(env: &Env) -> Address {
    env.register(crate::PriceAggregator, (Address::generate(env),))
}

/// Stores `items` on `aggregator` under a fresh token key with a
/// `SPREAD_TEST_BOUND` asset budget.
fn spread_key(env: &Env, aggregator: &Address, items: &[PriceSource]) -> PriceKey {
    let key = PriceKey::Token(Address::generate(env));
    env.as_contract(aggregator, || {
        registry::store_oracle(
            env,
            &key,
            &oracle(
                env,
                sources(env, items),
                SPREAD_TEST_BOUND,
                WAD / 2,
                2 * WAD,
            ),
        );
    });
    key
}

/// The error code of a status read of `key` on `aggregator`. Each key in `primed` is first
/// read strictly in the same session, so a nested read of it hits the
/// session cache.
fn status_code(
    env: &Env,
    aggregator: &Address,
    primed: &[&PriceKey],
    key: &PriceKey,
) -> Option<u32> {
    env.as_contract(aggregator, || {
        let mut session = Session::new(env);
        for prime in primed {
            let _ = resolve(&mut session, prime, 0);
        }
        resolve_status(&mut session, key, 0).error_code
    })
}

const STALE: Option<u32> = Some(OracleError::PriceFeedStale as u32);

/// E-1 fail-open side. `Twap(12)` at 300 s has its newest round now and its
/// oldest round 3300 s old. A spot leg may lag the newest round by the bound
/// and no more. The old check compared the oldest round and let a 6900 s lag
/// through.
#[test]
fn test_a_spot_leg_may_lag_the_newest_twap_round_by_the_spread_bound_only() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    let key = spread_key(
        &env,
        &aggregator,
        &[
            twap_leg(&env, 12, 300, 0),
            spot_leg(&env, &adapter, "SPOT", FeedNature::Market),
        ],
    );

    for (age, expected) in [
        (0, None),
        (MAX_LEG_AGE_SPREAD_SECONDS, None),
        (MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
        (6_900, STALE),
    ] {
        publish(&client, &env, "SPOT", WAD, age);
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "spot {age} s old"
        );
    }
}

/// E-1 fail-closed side. `Twap(12)` at 600 s has its oldest round 6600 s old,
/// inside its budget. A fresh spot leg prices beside it. The old check made
/// this pair stale while the spot leg was younger than 3000 s.
#[test]
fn test_a_twap_window_wider_than_the_spread_bound_prices_beside_a_fresh_spot_leg() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    let key = spread_key(
        &env,
        &aggregator,
        &[
            twap_leg(&env, 12, 600, 0),
            spot_leg(&env, &adapter, "SPOT", FeedNature::Market),
        ],
    );

    for (age, expected) in [
        (0, None),
        (2_999, None),
        (MAX_LEG_AGE_SPREAD_SECONDS, None),
        (MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
    ] {
        publish(&client, &env, "SPOT", WAD, age);
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "spot {age} s old"
        );
    }
}

/// The bound also holds the other way: a TWAP leg whose newest round lags a
/// fresh spot leg by more than the bound is stale, though every round is
/// inside the leg's own budget.
#[test]
fn test_a_twap_leg_whose_newest_round_lags_a_fresh_spot_leg_is_stale() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "SPOT", WAD, 0);

    for (newest_age, expected) in [
        (MAX_LEG_AGE_SPREAD_SECONDS, None),
        (MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
    ] {
        let key = spread_key(
            &env,
            &aggregator,
            &[
                spot_leg(&env, &adapter, "SPOT", FeedNature::Market),
                twap_leg(&env, 3, 300, newest_age),
            ],
        );
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "newest round {newest_age} s old"
        );
    }
}

/// Two spot market legs keep the bound on both sides of it. A Chainlink leg
/// and a multi-feed leg each carry one sample.
#[test]
fn test_two_spot_market_legs_keep_the_spread_bound() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    let (proxy, chainlink) = register_redstone_feed(&env);
    let key = spread_key(
        &env,
        &aggregator,
        &[
            PriceSource::Feed(chainlink_feed(&env, &proxy, SPREAD_TEST_BOUND)),
            spot_leg(&env, &adapter, "SPOT", FeedNature::Market),
        ],
    );
    chainlink.set_chainlink_round(&BytesN::from_array(&env, &[9; 32]), &WAD, &NOW);

    for (age, expected) in [
        (MAX_LEG_AGE_SPREAD_SECONDS, None),
        (MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
    ] {
        publish(&client, &env, "SPOT", WAD, age);
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "spot {age} s old"
        );
    }
}

/// A `Fundamental` leg stays exempt beside a TWAP market leg.
#[test]
fn test_a_fundamental_leg_may_lag_a_twap_market_leg_past_the_spread_bound() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "PEG", WAD, SPREAD_TEST_BOUND);
    let key = spread_key(
        &env,
        &aggregator,
        &[
            twap_leg(&env, 12, 300, 0),
            spot_leg(&env, &adapter, "PEG", FeedNature::Fundamental),
        ],
    );

    assert_eq!(status_code(&env, &aggregator, &[], &key), None);
}

/// A scaled leg enters the check with the older of the newest samples of its
/// factor and its quote. The quote key is a `Twap(12)` leg at 600 s. Its
/// oldest round, 6600 s old, does not enter the check. The quote prices from
/// the session cache when a batch reads it first.
#[test]
fn test_a_scaled_leg_over_a_twap_quote_enters_the_check_with_its_newest_samples() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    let quote = spread_key(&env, &aggregator, &[twap_leg(&env, 12, 600, 0)]);
    let scaled = PriceSource::Scaled(ScaledSource {
        factor: nature_feed(
            &env,
            &adapter,
            "RATIO",
            SPREAD_TEST_BOUND,
            FeedNature::Market,
        ),
        quote: quote.clone(),
        min_factor_wad: WAD / 2,
        max_factor_wad: 2 * WAD,
    });
    let key = spread_key(
        &env,
        &aggregator,
        &[scaled, spot_leg(&env, &adapter, "SPOT", FeedNature::Market)],
    );

    for (factor_age, spot_age, expected) in [
        (0, 0, None),
        (0, MAX_LEG_AGE_SPREAD_SECONDS, None),
        (0, MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
        (MAX_LEG_AGE_SPREAD_SECONDS, 0, None),
        (MAX_LEG_AGE_SPREAD_SECONDS + 1, 0, STALE),
    ] {
        publish(&client, &env, "RATIO", WAD, factor_age);
        publish(&client, &env, "SPOT", WAD, spot_age);
        let label = (factor_age, spot_age);
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "{label:?}"
        );
        assert_eq!(
            status_code(&env, &aggregator, &[&quote], &key),
            expected,
            "{label:?}"
        );
    }
}

/// A two-leg quote key passes on the older of its legs' newest samples. Its
/// TWAP leg's newest round is now and its spot leg is 3000 s old, so a scaled
/// leg over it enters the check 3000 s old.
#[test]
fn test_a_blended_quote_passes_on_the_older_newest_sample_of_its_legs() {
    let env = Env::default();
    at_now(&env);
    let aggregator = spread_aggregator(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "RATIO", WAD, 0);
    publish(&client, &env, "QSPOT", WAD, 3_000);
    let quote = spread_key(
        &env,
        &aggregator,
        &[
            twap_leg(&env, 12, 600, 0),
            spot_leg(&env, &adapter, "QSPOT", FeedNature::Market),
        ],
    );
    let scaled = PriceSource::Scaled(ScaledSource {
        factor: nature_feed(
            &env,
            &adapter,
            "RATIO",
            SPREAD_TEST_BOUND,
            FeedNature::Market,
        ),
        quote: quote.clone(),
        min_factor_wad: WAD / 2,
        max_factor_wad: 2 * WAD,
    });
    let key = spread_key(
        &env,
        &aggregator,
        &[scaled, spot_leg(&env, &adapter, "SPOT", FeedNature::Market)],
    );

    for (spot_age, expected) in [
        (3_000 + MAX_LEG_AGE_SPREAD_SECONDS, None),
        (3_000 + MAX_LEG_AGE_SPREAD_SECONDS + 1, STALE),
    ] {
        publish(&client, &env, "SPOT", WAD, spot_age);
        assert_eq!(
            status_code(&env, &aggregator, &[], &key),
            expected,
            "spot {spot_age} s old"
        );
        assert_eq!(
            status_code(&env, &aggregator, &[&quote], &key),
            expected,
            "spot {spot_age} s old"
        );
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

/// A skipped round is a transient gap, not a bad price. The samples still
/// cover the window, so the read must succeed rather than fail closed.
#[test]
fn test_twap_read_accepts_a_gapped_history_that_still_spans_the_window() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(GappedReflector, ());

    let feed = in_contract(&env, || resolve_twap(&env, &reflector, 3));

    assert_eq!(feed.price_wad, 2 * WAD);
    assert_eq!(feed.timestamp, NOW - TWAP_GAPPED_OLDER_AGE_SECS);
}

#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn test_twap_read_rejects_a_history_that_does_not_span_the_window() {
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
fn test_twap_read_truncates_more_samples_than_the_window_plus_current() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(LongHistoryReflector, ());
    in_contract(&env, || {
        let feed = resolve_twap(&env, &reflector, 2);
        assert_eq!(feed.price_wad, WAD);
        assert_eq!(feed.timestamp, NOW - TWAP_OLDER_AGE_SECS);
    });
}

/// Scripts `answer` on a fresh `ScriptedReflector` and reads it as
/// `Twap(records)`.
fn resolve_scripted(env: &Env, records: u32, answer: Vec<ReflectorPriceData>) -> PriceFeedRaw {
    let reflector = env.register(ScriptedReflector, ());
    ScriptedReflectorClient::new(env, &reflector).script(&REFLECTOR_RESOLUTION_SECS, &answer);
    resolve_twap(env, &reflector, records)
}

/// Sample `k` quotes `k + 1` dollars, so the newest three give a mean of 2
/// dollars, dated two rounds back.
#[test]
fn test_twap_read_uses_the_newest_records_samples_of_the_answer() {
    let env = Env::default();
    at_now(&env);
    let spacing = u64::from(REFLECTOR_RESOLUTION_SECS);
    in_contract(&env, || {
        // The counts equal `records`, exceed it by one (the public contract), and exceed
        // it by more.
        for count in [3, 4, 5, 12] {
            let feed = resolve_scripted(&env, 3, sample_run(&env, NOW, spacing, count, 1));
            assert_eq!(feed.price_wad, 2 * WAD, "{count} samples");
            assert_eq!(feed.timestamp, NOW - 2 * spacing, "{count} samples");
        }
    });
}

/// A bad tail past the first `records` samples does not fail the read.
#[test]
fn test_twap_read_ignores_the_samples_past_records() {
    let env = Env::default();
    at_now(&env);
    let spacing = u64::from(REFLECTOR_RESOLUTION_SECS);
    in_contract(&env, || {
        let mut answer = sample_run(&env, NOW, spacing, 3, 1);
        answer.push_back(sample(0, NOW - 3 * spacing));
        answer.push_back(sample(9, NOW - 3 * spacing));
        answer.push_back(sample(9, NOW));
        answer.push_back(sample(9, NOW + MAX_FUTURE_SKEW_SECONDS + 1));
        let feed = resolve_scripted(&env, 3, answer);
        assert_eq!(feed.price_wad, 2 * WAD);
        assert_eq!(feed.timestamp, NOW - 2 * spacing);
    });
}

/// Reflector fills a missing round with the round before it. A skip inside
/// the window gives `records` samples, one round older. A skip at the window
/// start adds an older round, which the read drops.
#[test]
fn test_twap_read_follows_the_answer_shapes_of_a_skipped_round() {
    let env = Env::default();
    at_now(&env);
    let spacing = u64::from(REFLECTOR_RESOLUTION_SECS);
    in_contract(&env, || {
        let skipped_inside = Vec::from_array(
            &env,
            [
                sample(1, NOW),
                sample(3, NOW - 2 * spacing),
                sample(5, NOW - 3 * spacing),
            ],
        );
        let feed = resolve_scripted(&env, 3, skipped_inside);
        assert_eq!(feed.price_wad, 3 * WAD);
        assert_eq!(feed.timestamp, NOW - 3 * spacing);

        let skipped_at_the_start = Vec::from_array(
            &env,
            [
                sample(1, NOW),
                sample(2, NOW - spacing),
                sample(3, NOW - 2 * spacing),
                sample(9, NOW - 5 * spacing),
            ],
        );
        let feed = resolve_scripted(&env, 3, skipped_at_the_start);
        assert_eq!(feed.price_wad, 2 * WAD);
        assert_eq!(feed.timestamp, NOW - 2 * spacing);
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
fn test_twap_read_rejects_a_used_sample_beyond_the_future_skew() {
    let env = Env::default();
    at_now(&env);
    let spacing = u64::from(REFLECTOR_RESOLUTION_SECS);
    let reflector = env.register(ScriptedReflector, ());
    let client = ScriptedReflectorClient::new(&env, &reflector);
    let feed = ReflectorFeedRef {
        contract: reflector,
        asset: OracleAssetRef::Symbol(Symbol::new(&env, "BTC")),
        read_mode: OracleReadMode::Twap(2),
    };
    for (skew, accepted) in [
        (MAX_FUTURE_SKEW_SECONDS, true),
        (MAX_FUTURE_SKEW_SECONDS + 1, false),
    ] {
        client.script(
            &REFLECTOR_RESOLUTION_SECS,
            &Vec::from_array(&env, [sample(1, NOW + skew), sample(3, NOW - spacing)]),
        );
        in_contract(&env, || {
            let mut session = Session::new(&env);
            let observation =
                crate::providers::reflector::read_reflector_source_impl(&mut session, &feed, 14);
            assert_eq!(observation.is_some(), accepted, "future skew {skew}");
            if let Some(observation) = observation {
                assert_eq!(observation.price_wad, 2 * WAD);
                assert_eq!(observation.timestamp, NOW - spacing);
            }
        });
    }
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
            NOW,
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
        // The depth error belongs to that walk, not to L1. A shallow read of L1 in the
        // same session must still price.
        let first_level = PriceKey::Ref(Symbol::new(&env, "L1"));
        assert!(resolve_nested(&mut session, &first_level, 0).is_ok());
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

/// `PriceAggregator::quotes` returns `to_status` output. An outcome carrying an
/// error collapses to `unusable()` plus its `error_code`. A consumer reads
/// `valid` to decide whether to act, so no other field of the errored outcome
/// leaks.
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
        PriceStatus {
            error_code: Some(OracleError::NoLastPrice as u32),
            ..PriceStatus::unusable()
        },
        "errored outcome must not leak any field"
    );
    assert!(!status.valid, "errored outcome reported valid");
    assert_eq!(status.final_wad, 0);
    assert_eq!(status.primary_wad, 0);
    assert_eq!(status.secondary_wad, 0);
}

/// A nested reference leg going stale appears on the dependent asset as
/// `Outcome::with_err`, which zeroes `stale` and `deviation`. `error_code` is
/// then the only field that separates it from a healthy price.
#[test]
fn to_status_reports_the_error_code_when_flags_are_zeroed() {
    let outcome = Outcome::with_err(OracleError::PriceFeedStale);

    let status = to_status(&outcome, None);

    assert!(!status.valid, "resolution error reported valid");
    assert!(
        !status.stale,
        "with_err zeroes the flags; this pins that shape"
    );
    assert!(
        !status.deviation,
        "with_err zeroes the flags; this pins that shape"
    );
    assert_eq!(
        status.error_code,
        Some(OracleError::PriceFeedStale as u32),
        "staleness failure must be readable from error_code alone"
    );
}

/// A clean outcome maps each field to its counterpart. Distinct primary and
/// secondary values catch a swapped mapping.
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

/// Without an error, the stale and deviation flags pass through unchanged for
/// the consumer to weigh.
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

/// Both cycle guards, driven directly. Each test pushes the key onto the
/// resolution stack, the state a re-entrant source produces, and checks that
/// the entry point refuses rather than recursing.
///
/// `MAX_RESOLUTION_DEPTH` bounds a chain. The stack bounds a cycle that fits inside the
/// depth cap.
#[test]
fn resolve_outcome_refuses_a_key_already_on_the_resolution_stack() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "BTC", ASSET_CEILING);
        let mut session = Session::new(&env);

        // Resolves normally while the stack is clear.
        assert!(resolve_outcome(&mut session, &key, 0, None).0.err.is_none());

        session.push_key(&key);
        let (outcome, oracle) = resolve_outcome(&mut session, &key, 0, None);
        assert_eq!(
            outcome.err,
            Some(OracleError::OracleCycleDetected),
            "a key already being resolved must not be resolved again"
        );
        assert!(
            oracle.is_none(),
            "a refused cycle must not report an oracle configuration"
        );
    });
}

#[test]
fn validate_cached_path_refuses_a_key_already_on_the_resolution_stack() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish(&client, &env, "BTC", 100 * WAD, 0);

    in_contract(&env, || {
        let key = single_feed_key(&env, &adapter, "BTC", ASSET_CEILING);
        let mut session = Session::new(&env);

        assert_eq!(validate_cached_path(&mut session, &key, 0), Ok(()));

        // A cache hit must not be trusted to skip the cycle check, because the cached
        // value was computed in some other resolution context.
        session.push_key(&key);
        assert_eq!(
            validate_cached_path(&mut session, &key, 0),
            Err(OracleError::OracleCycleDetected)
        );
    });
}
