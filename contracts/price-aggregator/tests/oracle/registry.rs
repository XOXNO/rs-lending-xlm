use super::*;
use crate::admin::{set_oracle, set_sanity_band, set_tolerance};
use crate::session::Session;
use common::errors::OracleError;
use common::oracle::providers::redstone::REDSTONE_DECIMALS;
use common::types::{
    AquariusLpSource, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, OracleTolerance, PriceSource, ProviderRef, ReflectorFeedRef, ScaledSource,
    MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String, Symbol, Vec};

use crate::test_support::{
    EmptyReflector, NonUsdReflector, StubXoxnoAdapter, TwapReflector, XOXNO_SUBMISSION_WINDOW_SECS,
};
use crate::PriceAggregator;
use common::constants::WAD;

fn oracle(env: &Env, decimals: u32) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Feed(FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: Address::generate(env),
            feed_id: String::from_str(env, "X"),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: 43_200,
    }));
    AssetOracle {
        asset_decimals: decimals,
        max_price_stale_seconds: 43_200,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX / 2,
    }
}

fn with_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

#[test]
fn test_round_trips_a_token_key() {
    let env = Env::default();
    with_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        store_oracle(&env, &key, &oracle(&env, 7));
        assert_eq!(get_oracle(&env, &key).unwrap().asset_decimals, 7);
    });
}

#[test]
fn test_round_trips_a_reference_key() {
    let env = Env::default();
    with_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "BTC"));
        store_oracle(&env, &key, &oracle(&env, 0));
        assert!(get_oracle(&env, &key).is_some());
    });
}

#[test]
fn test_token_and_reference_keys_do_not_alias() {
    let env = Env::default();
    with_contract(&env, || {
        let token = PriceKey::Token(Address::generate(&env));
        let reference = PriceKey::Ref(Symbol::new(&env, "BTC"));

        store_oracle(&env, &token, &oracle(&env, 7));
        assert!(get_oracle(&env, &reference).is_none());

        store_oracle(&env, &reference, &oracle(&env, 0));
        assert_eq!(get_oracle(&env, &token).unwrap().asset_decimals, 7);
        assert_eq!(get_oracle(&env, &reference).unwrap().asset_decimals, 0);
    });
}

#[test]
fn test_two_reference_symbols_do_not_alias() {
    let env = Env::default();
    with_contract(&env, || {
        let btc = PriceKey::Ref(Symbol::new(&env, "BTC"));
        let eth = PriceKey::Ref(Symbol::new(&env, "ETH"));
        store_oracle(&env, &btc, &oracle(&env, 0));
        assert!(get_oracle(&env, &eth).is_none());
    });
}

#[test]
fn test_an_unconfigured_key_resolves_to_none() {
    let env = Env::default();
    with_contract(&env, || {
        assert!(get_oracle(&env, &PriceKey::Token(Address::generate(&env))).is_none());
    });
}

#[test]
fn test_remove_disables_pricing_for_a_key() {
    let env = Env::default();
    with_contract(&env, || {
        let key = PriceKey::Token(Address::generate(&env));
        store_oracle(&env, &key, &oracle(&env, 7));
        remove_oracle(&env, &key);
        assert!(get_oracle(&env, &key).is_none());
    });
}

fn reflector_oracle(env: &Env, contract: &Address, decimals: u32) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Feed(reflector_leg(env, contract, decimals)));
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: 3_600,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,

        min_sanity_price_wad: TWAP_MEAN_WAD - TWAP_MEAN_WAD / 20,
        max_sanity_price_wad: TWAP_MEAN_WAD + TWAP_MEAN_WAD / 20,
    }
}

const TWAP_MEAN_WAD: i128 = 2 * common::constants::WAD;

fn reflector_leg(env: &Env, contract: &Address, decimals: u32) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Twap(2),
        }),
        decimals,
        max_stale_seconds: 3_600,
    }
}

#[test]
fn test_set_oracle_stores_a_config_whose_provider_facts_all_match() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(TwapReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(&env, key.clone(), reflector_oracle(&env, &reflector, 14));
        assert!(get_oracle(&env, &key).is_some());
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_set_oracle_rejects_a_reflector_whose_decimals_disagree() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(TwapReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(&env, key, reflector_oracle(&env, &reflector, 8));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_set_oracle_rejects_a_reflector_that_does_not_quote_usd() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(NonUsdReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(&env, key, reflector_oracle(&env, &reflector, 14));
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_set_oracle_attests_the_factor_leg_of_a_scaled_source() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(TwapReflector, ());
        let quote = PriceKey::Ref(Symbol::new(&env, "BTC"));
        store_oracle(&env, &quote, &reflector_oracle(&env, &reflector, 14));

        let mut sources = Vec::new(&env);
        sources.push_back(PriceSource::Scaled(ScaledSource {
            factor: reflector_leg(&env, &reflector, 8),
            quote,
            min_factor_wad: 1,
            max_factor_wad: 10 * common::constants::WAD,
        }));
        let mut cfg = reflector_oracle(&env, &reflector, 14);
        cfg.sources = sources;

        set_oracle(&env, PriceKey::Token(Address::generate(&env)), cfg);
    });
}

fn xoxno_oracle(env: &Env, contract: &Address, max_stale: u64) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Feed(FeedSource {
        provider: ProviderRef::Xoxno(MultiFeedRef {
            contract: contract.clone(),
            feed_id: String::from_str(env, "BTC/USD"),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }));
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: max_stale.max(XOXNO_SUBMISSION_WINDOW_SECS),
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: WAD - WAD / 20,
        max_sanity_price_wad: WAD + WAD / 20,
    }
}

#[test]
fn test_set_oracle_accepts_a_window_matching_the_xoxno_adapter() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let adapter = env.register(StubXoxnoAdapter, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(
            &env,
            key.clone(),
            xoxno_oracle(&env, &adapter, XOXNO_SUBMISSION_WINDOW_SECS),
        );
        assert!(get_oracle(&env, &key).is_some());
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_set_oracle_rejects_a_window_tighter_than_the_xoxno_adapter_allows() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let adapter = env.register(StubXoxnoAdapter, ());
        set_oracle(
            &env,
            PriceKey::Token(Address::generate(&env)),
            xoxno_oracle(&env, &adapter, XOXNO_SUBMISSION_WINDOW_SECS - 1),
        );
    });
}

#[test]
fn test_revalidation_touches_only_the_keys_that_actually_depend_on_the_change() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let stranded = PriceKey::Ref(Symbol::new(&env, "STRANDED"));
        store_oracle(&env, &stranded, &oracle(&env, 8));

        let reflector = env.register(TwapReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(&env, key.clone(), reflector_oracle(&env, &reflector, 14));

        assert!(get_oracle(&env, &key).is_some());
        assert!(
            get_oracle(&env, &stranded).is_some(),
            "the unrelated key is untouched, not repaired and not removed"
        );
    });
}

const REBAND_DELTA_WAD: i128 = TWAP_MEAN_WAD * 7 / 100;

#[test]
fn test_sanity_band_is_editable_while_the_price_is_unreadable() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(EmptyReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        store_oracle(&env, &key, &reflector_oracle(&env, &reflector, 14));

        set_sanity_band(
            &env,
            key.clone(),
            TWAP_MEAN_WAD - REBAND_DELTA_WAD,
            TWAP_MEAN_WAD + REBAND_DELTA_WAD,
        );

        let stored = get_oracle(&env, &key).unwrap();
        assert_eq!(
            stored.min_sanity_price_wad,
            TWAP_MEAN_WAD - REBAND_DELTA_WAD
        );
        assert_eq!(
            stored.max_sanity_price_wad,
            TWAP_MEAN_WAD + REBAND_DELTA_WAD
        );
    });
}

#[test]
fn test_tolerance_is_editable_while_the_price_is_unreadable() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(EmptyReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        store_oracle(&env, &key, &reflector_oracle(&env, &reflector, 14));

        set_tolerance(
            &env,
            key.clone(),
            OracleTolerance {
                upper_ratio_bps: 10_200,
                lower_ratio_bps: 9_804,
            },
        );

        assert_eq!(
            get_oracle(&env, &key).unwrap().tolerance.upper_ratio_bps,
            10_200
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn test_probe_still_rejects_a_structurally_broken_config() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(EmptyReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        let mut broken = reflector_oracle(&env, &reflector, 14);
        broken.sources = Vec::new(&env);
        store_oracle(&env, &key, &broken);

        set_sanity_band(
            &env,
            key,
            TWAP_MEAN_WAD - REBAND_DELTA_WAD,
            TWAP_MEAN_WAD + REBAND_DELTA_WAD,
        );
    });
}

fn lp_fixture(
    env: &Env,
    kind: &str,
    reserve_a: u128,
    reserve_b: u128,
    total_shares: u128,
) -> (Address, Address, Address, Address, Address) {
    let issuer = Address::generate(env);
    let token_a = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let share = env.register_stellar_asset_contract_v2(issuer).address();
    let plane = env.register(
        crate::test_support::MockAquariusPlane,
        (Symbol::new(env, kind), reserve_a, reserve_b),
    );
    let pool = env.register(
        crate::test_support::MockAquariusPool,
        (
            plane.clone(),
            share.clone(),
            token_a.clone(),
            token_b.clone(),
            total_shares,
        ),
    );
    let pool_client = crate::test_support::MockAquariusPoolClient::new(env, &pool);
    pool_client.set_pool_type(&Symbol::new(
        env,
        if kind == "standard" {
            "constant_product"
        } else {
            kind
        },
    ));
    pool_client.set_reserves(&reserve_a, &reserve_b);
    (pool, plane, share, token_a, token_b)
}

fn lp_oracle(
    env: &Env,
    pool: &Address,
    plane: &Address,
    token_a: &Address,
    token_b: &Address,
    key_a: PriceKey,
    key_b: PriceKey,
) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::AquariusLp(common::types::AquariusLpSource {
        pool: pool.clone(),
        plane: plane.clone(),
        token_a: token_a.clone(),
        token_b: token_b.clone(),
        key_a,
        key_b,
        reserve_a_decimals: 7,
        reserve_b_decimals: 7,
        min_pool_value_wad: WAD,
    }));
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 43_200,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: WAD,
        max_sanity_price_wad: 3 * WAD,
    }
}

fn dollar_underlyings(env: &Env, token_a: &Address, token_b: &Address) -> (PriceKey, PriceKey) {
    let (adapter, client) = crate::test_support::register_redstone_feed(env);
    let ts = env.ledger().timestamp() * 1_000;
    let mut keys = Vec::new(env);
    for (feed, token) in [("UA", token_a), ("UB", token_b)] {
        client.set_price_data(&String::from_str(env, feed), &WAD, &ts, &ts);
        let key = PriceKey::Token(token.clone());
        let mut sources = Vec::new(env);
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: adapter.clone(),
                feed_id: String::from_str(env, feed),
                nature: FeedNature::Fundamental,
            }),
            decimals: 8,
            max_stale_seconds: 43_200,
        }));
        set_oracle(
            env,
            key.clone(),
            AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 43_200,
                sources,
                tolerance: OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: WAD * 99 / 100,
                max_sanity_price_wad: WAD * 101 / 100,
            },
        );
        keys.push_back(key);
    }
    (keys.get_unchecked(0), keys.get_unchecked(1))
}

fn listable_lp(
    env: &Env,
    kind: &str,
    reserve_a: u128,
    reserve_b: u128,
    total_shares: u128,
) -> (Address, Address, Address, AssetOracle) {
    let (pool, plane, share, token_a, token_b) =
        lp_fixture(env, kind, reserve_a, reserve_b, total_shares);
    let (key_a, key_b) = dollar_underlyings(env, &token_a, &token_b);
    let oracle = lp_oracle(env, &pool, &plane, &token_a, &token_b, key_a, key_b);
    (pool, plane, share, oracle)
}

fn lp_of(oracle: &AssetOracle) -> AquariusLpSource {
    match oracle.sources.get_unchecked(0) {
        PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => lp,
        _ => unreachable!(),
    }
}

fn set_lp(oracle: &mut AssetOracle, lp: AquariusLpSource) {
    oracle.sources.set(0, PriceSource::AquariusLp(lp));
}

fn set_lp_min_pool_value(oracle: &mut AssetOracle, min_pool_value_wad: i128) {
    let mut lp = lp_of(oracle);
    lp.min_pool_value_wad = min_pool_value_wad;
    set_lp(oracle, lp);
}

#[test]
fn test_set_oracle_lists_a_constant_product_lp_and_prices_it() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);

        let mut session = Session::new(&env);
        let feed = crate::engine::resolve(&mut session, &key, 0);
        assert_eq!(feed.price_wad, 2 * WAD);
        assert_eq!(feed.asset_decimals, 7);
    });
}

fn stable_lp_oracle(
    env: &Env,
    pool: &Address,
    plane: &Address,
    token_a: &Address,
    token_b: &Address,
    key_a: PriceKey,
    key_b: PriceKey,
) -> AssetOracle {
    let mut oracle = lp_oracle(env, pool, plane, token_a, token_b, key_a, key_b);
    let lp = lp_of(&oracle);
    oracle.sources.set(0, PriceSource::AquariusStableLp(lp));
    oracle
}

const STABLE_AMP: u128 = 1_500;

const BALANCED_STABLE_UNITS: u128 = 10_000_000_000;

const BALANCED_STABLE_SHARES: i128 = 1_000;

fn balanced_stable_lp(env: &Env) -> (Address, Address, Address, AssetOracle) {
    let (pool, plane, share, token_a, token_b) = lp_fixture(
        env,
        "stable",
        BALANCED_STABLE_UNITS,
        BALANCED_STABLE_UNITS,
        BALANCED_STABLE_UNITS,
    );
    crate::test_support::MockAquariusPoolClient::new(env, &pool).set_amp(&STABLE_AMP);
    let (key_a, key_b) = dollar_underlyings(env, &token_a, &token_b);
    let oracle = stable_lp_oracle(env, &pool, &plane, &token_a, &token_b, key_a, key_b);
    (pool, plane, share, oracle)
}

#[test]
fn test_set_oracle_lists_a_stableswap_lp_and_prices_it() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, plane, share, token_a, token_b) = lp_fixture(
            &env,
            "stable",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        crate::test_support::MockAquariusPoolClient::new(&env, &pool).set_amp(&1500);
        let (key_a, key_b) = dollar_underlyings(&env, &token_a, &token_b);
        let oracle = stable_lp_oracle(&env, &pool, &plane, &token_a, &token_b, key_a, key_b);
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);

        let mut session = Session::new(&env);
        let feed = crate::engine::resolve(&mut session, &key, 0);
        assert!(
            (feed.price_wad - 2 * WAD).abs() < WAD / 1_000,
            "stable fair price off $2: {}",
            feed.price_wad
        );
        assert_eq!(feed.asset_decimals, 7);
    });
}

#[test]
#[should_panic]
fn test_stable_lp_rejects_a_constant_product_pool() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, plane, share, token_a, token_b) = lp_fixture(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        crate::test_support::MockAquariusPoolClient::new(&env, &pool).set_amp(&1500);
        let (key_a, key_b) = dollar_underlyings(&env, &token_a, &token_b);
        let oracle = stable_lp_oracle(&env, &pool, &plane, &token_a, &token_b, key_a, key_b);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic]
fn test_set_oracle_rejects_a_reserve_token_mismatch() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let PriceSource::AquariusLp(mut lp) = oracle.sources.get_unchecked(0) else {
            unreachable!()
        };
        lp.token_a = Address::generate(&env);
        oracle.sources.set(0, PriceSource::AquariusLp(lp));
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic]
fn test_set_oracle_rejects_pool_invariant_that_does_not_match_the_plane() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_reserves(&20_000_000_000, &10_000_000_000);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
fn test_set_oracle_accepts_reserve_ratio_drift_that_preserves_the_invariant() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_reserves(&20_000_000_000, &5_000_000_000);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic]
fn test_set_oracle_rejects_insufficient_pool_value() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        set_lp_min_pool_value(&mut oracle, 2_001 * WAD);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
fn test_lp_read_rejects_pool_plane_drift() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_plane(&Address::generate(&env));

        let mut session = Session::new(&env);
        assert!(!crate::engine::resolve_status(&mut session, &key, 0).valid);
    });
}

#[test]
fn test_lp_read_rejects_pool_type_drift() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_pool_type(&Symbol::new(&env, "stable"));

        let mut session = Session::new(&env);
        assert!(!crate::engine::resolve_status(&mut session, &key, 0).valid);
    });
}

#[test]
fn test_lp_read_rejects_pool_token_drift() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_tokens(&Address::generate(&env), &Address::generate(&env));

        let mut session = Session::new(&env);
        assert!(!crate::engine::resolve_status(&mut session, &key, 0).valid);
    });
}

#[test]
fn test_lp_read_rejects_share_token_drift() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_share(&Address::generate(&env));

        let mut session = Session::new(&env);
        assert!(!crate::engine::resolve_status(&mut session, &key, 0).valid);
    });
}

#[test]
fn test_lp_read_rejects_liquidity_below_floor() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        set_lp_min_pool_value(&mut oracle, 1_000 * WAD);
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        crate::test_support::MockAquariusPlaneClient::new(&env, &plane)
            .set_reserves(&100_000_000, &100_000_000);

        let mut session = Session::new(&env);
        assert!(!crate::engine::resolve_status(&mut session, &key, 0).valid);
    });
}

#[test]
#[should_panic]
fn test_set_oracle_rejects_a_non_standard_pool() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) = listable_lp(
            &env,
            "stable",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
fn test_lp_oracle_needs_no_tolerance_band() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        oracle.tolerance = OracleTolerance {
            upper_ratio_bps: 0,
            lower_ratio_bps: 0,
        };
        set_oracle(&env, key.clone(), oracle);

        let mut session = Session::new(&env);
        assert_eq!(
            crate::engine::resolve(&mut session, &key, 0).price_wad,
            2 * WAD
        );
    });
}

#[test]
#[should_panic]
fn test_set_tolerance_refuses_an_lp_oracle() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);
        set_tolerance(
            &env,
            key,
            OracleTolerance {
                upper_ratio_bps: 10_500,
                lower_ratio_bps: 9_524,
            },
        );
    });
}

#[test]
#[should_panic]
fn test_set_oracle_rejects_a_pool_with_zero_total_shares() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) =
            listable_lp(&env, "standard", 10_000_000_000, 10_000_000_000, 0);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic]
fn test_set_oracle_refuses_an_lp_that_cannot_price_at_listing() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        oracle.min_sanity_price_wad = 5 * WAD;
        oracle.max_sanity_price_wad = 6 * WAD;
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
fn test_a_non_lp_config_still_lands_when_its_price_is_out_of_band() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (adapter, client) = crate::test_support::register_redstone_feed(&env);
        let ts = env.ledger().timestamp() * 1_000;
        client.set_price_data(&String::from_str(&env, "OOB"), &WAD, &ts, &ts);
        let key = PriceKey::Token(Address::generate(&env));
        let mut sources = Vec::new(&env);
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: adapter,
                feed_id: String::from_str(&env, "OOB"),
                nature: FeedNature::Fundamental,
            }),
            decimals: 8,
            max_stale_seconds: 43_200,
        }));
        set_oracle(
            &env,
            key.clone(),
            AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 43_200,
                sources,
                tolerance: OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: 5 * WAD,
                max_sanity_price_wad: 11 * WAD / 2,
            },
        );
        assert!(get_oracle(&env, &key).is_some(), "config must be stored");
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #231)")]
fn test_set_tolerance_probes_the_stored_config_before_committing() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let reflector = env.register(EmptyReflector, ());
        let key = PriceKey::Token(Address::generate(&env));
        let mut broken = reflector_oracle(&env, &reflector, 14);
        broken.sources = Vec::new(&env);
        store_oracle(&env, &key, &broken);

        set_tolerance(
            &env,
            key,
            OracleTolerance {
                upper_ratio_bps: 10_200,
                lower_ratio_bps: 9_804,
            },
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_set_oracle_rejects_a_redstone_leg_that_is_not_eight_decimals() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (adapter, client) = crate::test_support::register_redstone_feed(&env);
        let ts = env.ledger().timestamp() * 1_000;
        client.set_price_data(&String::from_str(&env, "RS"), &WAD, &ts, &ts);

        let mut sources = Vec::new(&env);
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: adapter,
                feed_id: String::from_str(&env, "RS"),
                nature: FeedNature::Fundamental,
            }),
            decimals: REDSTONE_DECIMALS + 1,
            max_stale_seconds: 43_200,
        }));

        set_oracle(
            &env,
            PriceKey::Token(Address::generate(&env)),
            AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 43_200,
                sources,
                tolerance: OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: WAD * 99 / 100,
                max_sanity_price_wad: WAD * 101 / 100,
            },
        );
    });
}

#[test]
fn test_a_xoxno_leg_prices_through_the_shared_multi_feed_reader() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let adapter = env.register(StubXoxnoAdapter, ());
        let key = PriceKey::Token(Address::generate(&env));
        set_oracle(
            &env,
            key.clone(),
            xoxno_oracle(&env, &adapter, XOXNO_SUBMISSION_WINDOW_SECS),
        );

        let mut session = Session::new(&env);
        assert_eq!(crate::engine::resolve(&mut session, &key, 0).price_wad, WAD);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn test_set_oracle_rejects_a_pool_reporting_another_token_in_the_first_slot() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let impostor = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();
        crate::test_support::MockAquariusPoolClient::new(&env, &pool)
            .set_tokens(&impostor, &lp_of(&oracle).token_b);

        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn test_set_oracle_rejects_reserve_decimals_the_token_does_not_report() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let mut lp = lp_of(&oracle);
        lp.reserve_a_decimals -= 1;
        set_lp(&mut oracle, lp);

        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #234)")]
fn test_set_oracle_rejects_a_pool_with_one_empty_reserve() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) =
            listable_lp(&env, "standard", 0, 10_000_000_000, 10_000_000_000);
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

#[test]
fn test_set_oracle_accepts_a_pool_worth_exactly_its_liquidity_floor() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, mut oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        set_lp_min_pool_value(&mut oracle, 2_000 * WAD);
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), oracle);

        assert!(get_oracle(&env, &key).is_some());
    });
}

fn deepen_leg(env: &Env, leg: &PriceKey, leaf: &str) {
    let original = get_oracle(env, leg).unwrap();
    let PriceSource::Feed(feed) = original.sources.get_unchecked(0) else {
        unreachable!()
    };
    let leaf = PriceKey::Ref(Symbol::new(env, leaf));
    store_oracle(env, &leaf, &original);

    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor: feed,
        quote: leaf,
        min_factor_wad: 1,
        max_factor_wad: 10 * WAD,
    }));
    let mut nested = original.clone();
    nested.sources = sources;
    store_oracle(env, leg, &nested);
}

fn assert_deepened_leg_exhausts_the_cap(deepen_key_a: bool) {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) = listable_lp(
            &env,
            "standard",
            10_000_000_000,
            10_000_000_000,
            10_000_000_000,
        );
        let lp = lp_of(&oracle);
        let key = PriceKey::Token(share);
        store_oracle(&env, &key, &oracle);

        let leg = if deepen_key_a { &lp.key_a } else { &lp.key_b };
        deepen_leg(&env, leg, "LEAF");

        let mut session = Session::new(&env);
        assert_eq!(
            crate::providers::aquarius::read(&mut session, &key, &lp, 7, MAX_RESOLUTION_DEPTH - 1)
                .err(),
            Some(OracleError::OracleDepthExceeded)
        );
    });
}

#[test]
fn test_the_first_lp_leg_is_resolved_one_level_below_the_lp() {
    assert_deepened_leg_exhausts_the_cap(true);
}

#[test]
fn test_the_second_lp_leg_is_resolved_one_level_below_the_lp() {
    assert_deepened_leg_exhausts_the_cap(false);
}

#[test]
#[should_panic(expected = "Error(Contract, #234)")]
fn test_set_oracle_rejects_a_stable_plane_whose_invariant_drifts_past_the_cap() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, plane, share, oracle) = balanced_stable_lp(&env);
        crate::test_support::MockAquariusPlaneClient::new(&env, &plane)
            .set_reserves(&BALANCED_STABLE_UNITS, &(BALANCED_STABLE_UNITS * 101 / 100));

        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

fn priced_stable_lp(env: &Env) -> (PriceKey, AquariusLpSource, i128) {
    let (_pool, _plane, share, oracle) = balanced_stable_lp(env);
    let key = PriceKey::Token(share);
    let lp = lp_of(&oracle);

    let mut session = Session::new(env);
    let (observation, _) = crate::providers::aquarius::read_stable(&mut session, &key, &lp, 7, 0)
        .expect("a balanced stable pool must price")
        .expect("a balanced stable pool must report an observation");
    let pool_value_wad = observation.price_wad * BALANCED_STABLE_SHARES;
    (key, lp, pool_value_wad)
}

#[test]
fn test_a_stable_pool_worth_exactly_its_liquidity_floor_still_prices() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key, mut lp, pool_value_wad) = priced_stable_lp(&env);
        lp.min_pool_value_wad = pool_value_wad;

        let mut session = Session::new(&env);
        assert!(crate::providers::aquarius::read_stable(&mut session, &key, &lp, 7, 0).is_ok());
    });
}

#[test]
fn test_a_stable_pool_one_wei_under_its_liquidity_floor_is_refused() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key, mut lp, pool_value_wad) = priced_stable_lp(&env);
        lp.min_pool_value_wad = pool_value_wad + 1;

        let mut session = Session::new(&env);
        assert_eq!(
            crate::providers::aquarius::read_stable(&mut session, &key, &lp, 7, 0).err(),
            Some(OracleError::InsufficientAquariusLiquidity)
        );
    });
}

fn assert_deepened_stable_leg_exhausts_the_cap(deepen_key_a: bool) {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (_pool, _plane, share, oracle) = balanced_stable_lp(&env);
        let lp = lp_of(&oracle);
        let key = PriceKey::Token(share);

        let leg = if deepen_key_a { &lp.key_a } else { &lp.key_b };
        deepen_leg(&env, leg, "LEAF");

        let mut session = Session::new(&env);
        assert_eq!(
            crate::providers::aquarius::read_stable(
                &mut session,
                &key,
                &lp,
                7,
                MAX_RESOLUTION_DEPTH - 1
            )
            .err(),
            Some(OracleError::OracleDepthExceeded)
        );
    });
}

#[test]
fn test_the_first_stable_lp_leg_is_resolved_one_level_below_the_lp() {
    assert_deepened_stable_leg_exhausts_the_cap(true);
}

#[test]
fn test_the_second_stable_lp_leg_is_resolved_one_level_below_the_lp() {
    assert_deepened_stable_leg_exhausts_the_cap(false);
}
