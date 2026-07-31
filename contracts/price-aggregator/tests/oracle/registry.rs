use super::*;
use common::types::{
    FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef, OracleReadMode,
    OracleTolerance, PriceSource, ProviderKind, ProviderRef, ReflectorFeedRef, ScaledSource,
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
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: Address::generate(env),
            feed_id: String::from_str(env, "X"),
            kind: ProviderKind::RedStone,
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
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: contract.clone(),
            feed_id: String::from_str(env, "BTC/USD"),
            kind: ProviderKind::Xoxno,
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

/// Builds a listable Aquarius LP market: two 7-dp reserve SACs, a 7-dp share
/// SAC, a plane mirroring `(reserve_a, reserve_b)` and a pool wired to them.
fn lp_fixture(
    env: &Env,
    kind: &str,
    reserve_a: u128,
    reserve_b: u128,
    total_shares: u128,
) -> (Address, Address, Address) {
    let issuer = Address::generate(env);
    let token_a = env.register_stellar_asset_contract_v2(issuer.clone()).address();
    let token_b = env.register_stellar_asset_contract_v2(issuer.clone()).address();
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
            token_a,
            token_b,
            total_shares,
        ),
    );
    (pool, plane, share)
}

fn lp_oracle(
    env: &Env,
    pool: &Address,
    plane: &Address,
    key_a: PriceKey,
    key_b: PriceKey,
    share_decimals: u32,
) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::LpShare(common::types::LpShareSource {
        pool: pool.clone(),
        plane: plane.clone(),
        kind: common::types::PoolKind::ConstantProduct,
        key_a,
        key_b,
        reserve_a_decimals: 7,
        reserve_b_decimals: 7,
        share_decimals,
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

/// Lists both underlyings at $1 through the production path and returns their keys.
fn dollar_underlyings(env: &Env) -> (PriceKey, PriceKey) {
    let (adapter, client) = crate::test_support::register_redstone_feed(env);
    let ts = env.ledger().timestamp() * 1_000;
    let mut keys = Vec::new(env);
    for feed in ["UA", "UB"] {
        client.set_price_data(&String::from_str(env, feed), &WAD, &ts, &ts);
        let key = PriceKey::Token(Address::generate(env));
        let mut sources = Vec::new(env);
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::MultiFeed(MultiFeedRef {
                contract: adapter.clone(),
                feed_id: String::from_str(env, feed),
                kind: ProviderKind::RedStone,
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

// The production listing path must accept a constant-product LP: an LpShare is
// always a sole source and has no smoothing window of its own, so the spot-only
// gate must not apply to it. Also proves attest_lp_binding is reachable.
#[test]
fn test_set_oracle_lists_a_constant_product_lp_and_prices_it() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        // 1000 + 1000 whole tokens at $1, 1000 whole shares => $2.00 per share.
        let (pool, plane, share) =
            lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        let key = PriceKey::Token(share);
        set_oracle(
            &env,
            key.clone(),
            lp_oracle(&env, &pool, &plane, key_a, key_b, 7),
        );

        let mut session = Session::new(&env);
        let feed = crate::engine::resolve(&mut session, &key, 0);
        assert_eq!(feed.price_wad, 2 * WAD);
        assert_eq!(feed.asset_decimals, 7);
    });
}

// share_decimals is attested against the share token itself: a typo would
// rescale the price by 10^k while still landing inside a generous band.
#[test]
#[should_panic]
fn test_set_oracle_rejects_lp_share_decimals_that_disagree() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) =
            lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        set_oracle(
            &env,
            PriceKey::Token(share),
            lp_oracle(&env, &pool, &plane, key_a, key_b, 11),
        );
    });
}

// A stable/concentrated pool mislisted as constant-product is structural: its
// plane row is not [reserve0, reserve1], so listing must fail, not go dead.
#[test]
#[should_panic]
fn test_set_oracle_rejects_a_non_standard_pool() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) =
            lp_fixture(&env, "stable", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        set_oracle(
            &env,
            PriceKey::Token(share),
            lp_oracle(&env, &pool, &plane, key_a, key_b, 7),
        );
    });
}

// An LpShare is always the sole source, so no primary/anchor band is ever read:
// a zero tolerance must list, and set_tolerance must refuse rather than store a
// band that implies a cross-check the oracle does not perform.
#[test]
fn test_lp_oracle_needs_no_tolerance_band() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) =
            lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        let key = PriceKey::Token(share);
        let mut oracle = lp_oracle(&env, &pool, &plane, key_a, key_b, 7);
        oracle.tolerance = OracleTolerance {
            upper_ratio_bps: 0,
            lower_ratio_bps: 0,
        };
        set_oracle(&env, key.clone(), oracle);

        let mut session = Session::new(&env);
        assert_eq!(crate::engine::resolve(&mut session, &key, 0).price_wad, 2 * WAD);
    });
}

#[test]
#[should_panic]
fn test_set_tolerance_refuses_an_lp_oracle() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) =
            lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        let key = PriceKey::Token(share);
        set_oracle(&env, key.clone(), lp_oracle(&env, &pool, &plane, key_a, key_b, 7));
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

// A pool that cannot report a positive supply is unpriceable by construction,
// so it must be refused at listing rather than stored as a dead market.
#[test]
#[should_panic]
fn test_set_oracle_rejects_a_pool_with_zero_total_shares() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) = lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 0);
        set_oracle(
            &env,
            PriceKey::Token(share),
            lp_oracle(&env, &pool, &plane, key_a, key_b, 7),
        );
    });
}

// Listing an LP whose price cannot be formed is a configuration error: there is
// no incident to work around when adding a new asset, so the write is refused
// rather than stored as a market that is dead on its first read.
#[test]
#[should_panic]
fn test_set_oracle_refuses_an_lp_that_cannot_price_at_listing() {
    let env = Env::default();
    env.ledger().set_timestamp(1_000_000);
    with_contract(&env, || {
        let (key_a, key_b) = dollar_underlyings(&env);
        let (pool, plane, share) =
            lp_fixture(&env, "standard", 10_000_000_000, 10_000_000_000, 10_000_000_000);
        let mut oracle = lp_oracle(&env, &pool, &plane, key_a, key_b, 7);
        // The pool prices at $2.00/share; this band excludes it.
        oracle.min_sanity_price_wad = 5 * WAD;
        oracle.max_sanity_price_wad = 6 * WAD;
        set_oracle(&env, PriceKey::Token(share), oracle);
    });
}

// The strict listing probe must not leak into the incident levers: an ordinary
// asset whose live price is out of band still lands, so it stays repairable.
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
            provider: ProviderRef::MultiFeed(MultiFeedRef {
                contract: adapter,
                feed_id: String::from_str(&env, "OOB"),
                kind: ProviderKind::RedStone,
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
                // Live price is $1.00, this band excludes it.
                min_sanity_price_wad: 5 * WAD,
                max_sanity_price_wad: 11 * WAD / 2,
            },
        );
        assert!(get_oracle(&env, &key).is_some(), "config must be stored");
    });
}
