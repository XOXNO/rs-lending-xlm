use super::*;
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef,
    OracleReadMode, OracleTolerance, ProviderKind, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Symbol};

use crate::PriceAggregator;

fn twap_feed(env: &Env, contract: &Address) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Twap(3),
        }),
        decimals: 14,
        max_stale_seconds: 3_600,
    }
}

fn spot_feed(env: &Env, contract: &Address) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Spot,
        }),
        decimals: 14,
        max_stale_seconds: 3_600,
    }
}

fn nav_feed(env: &Env, contract: &Address, feed: &str) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: contract.clone(),
            feed_id: String::from_str(env, feed),
            kind: ProviderKind::RedStone,
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: 86_400,
    }
}

fn oracle_of(_env: &Env, sources: Vec<PriceSource>) -> AssetOracle {
    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: 86_400,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX / 2,
    }
}

fn one(env: &Env, source: PriceSource) -> Vec<PriceSource> {
    let mut sources = Vec::new(env);
    sources.push_back(source);
    sources
}

fn scaled_onto(env: &Env, adapter: &Address, quote: PriceKey) -> PriceSource {
    PriceSource::Scaled(ScaledSource {
        factor: nav_feed(env, adapter, "ratio"),
        quote,
        min_factor_wad: 1,
        max_factor_wad: i128::MAX,
    })
}

fn with_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

#[test]
fn test_feed_source_sits_at_depth_zero_and_names_one_domain() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    with_contract(&env, || {
        let mut cache = ResolutionContext::new(&env);
        let source = PriceSource::Feed(twap_feed(&env, &reflector));
        let props = properties_of_source(&mut cache, &source, 0);

        assert_eq!(props.depth, 0);
        assert_eq!(props.trust.len(), 1);
        assert!(!props.has_unsmoothed_market_leg);
        assert_eq!(props.loosest_max_stale_seconds, 3_600);
    });
}

#[test]
fn test_scaled_source_folds_in_its_quote() {
    // The SolvBTC shape: a fundamental ratio scaled by a reference price. The
    // composite inherits the quote's trust domain and reports the looser of the
    // two staleness bounds, which is what the config rule compares against the
    // asset-level ceiling.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    with_contract(&env, || {
        let btc = PriceKey::Ref(Symbol::new(&env, "BTC"));
        registry::set_oracle(
            &env,
            &btc,
            &oracle_of(
                &env,
                one(&env, PriceSource::Feed(twap_feed(&env, &reflector))),
            ),
        );

        let mut cache = ResolutionContext::new(&env);
        let scaled = PriceSource::Scaled(ScaledSource {
            factor: nav_feed(&env, &adapter, "SolvBTC_FUNDAMENTAL"),
            quote: btc,
            min_factor_wad: 10i128.pow(18),
            max_factor_wad: 2 * 10i128.pow(18),
        });
        let props = properties_of_source(&mut cache, &scaled, 0);

        assert_eq!(props.trust.len(), 2, "ratio publisher plus quote publisher");
        assert_eq!(props.depth, 1, "one composition level");
        assert!(
            !props.has_unsmoothed_market_leg,
            "a fundamental ratio over a TWAP quote has nothing trading can move"
        );
        assert_eq!(props.loosest_max_stale_seconds, 86_400);
    });
}

#[test]
fn test_market_quote_taints_a_fundamental_factor() {
    // A spot quote is movable by trading, and the defect must reach the
    // composite even though the factor itself is safe.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    with_contract(&env, || {
        let btc = PriceKey::Ref(Symbol::new(&env, "BTC"));
        registry::set_oracle(
            &env,
            &btc,
            &oracle_of(
                &env,
                one(&env, PriceSource::Feed(spot_feed(&env, &reflector))),
            ),
        );

        let mut cache = ResolutionContext::new(&env);
        let scaled = scaled_onto(&env, &adapter, btc);
        assert!(properties_of_source(&mut cache, &scaled, 0).has_unsmoothed_market_leg);
    });
}

#[test]
#[should_panic]
fn test_a_self_quoting_scaled_source_is_caught_as_a_cycle() {
    // The guard is pushed before the config is read, so re-entry is seen. A
    // guard installed after resolution would never fire.
    let env = Env::default();
    let adapter = Address::generate(&env);

    with_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "LOOP"));
        let source = scaled_onto(&env, &adapter, key.clone());
        registry::set_oracle(&env, &key, &oracle_of(&env, one(&env, source)));

        let mut cache = ResolutionContext::new(&env);
        let _ = properties_of_key(&mut cache, &key, 0);
    });
}

#[test]
#[should_panic]
fn test_a_two_key_cycle_is_caught() {
    let env = Env::default();
    let adapter = Address::generate(&env);

    with_contract(&env, || {
        let a = PriceKey::Ref(Symbol::new(&env, "A"));
        let b = PriceKey::Ref(Symbol::new(&env, "B"));

        let a_source = scaled_onto(&env, &adapter, b.clone());
        let b_source = scaled_onto(&env, &adapter, a.clone());
        registry::set_oracle(&env, &a, &oracle_of(&env, one(&env, a_source)));
        registry::set_oracle(&env, &b, &oracle_of(&env, one(&env, b_source)));

        let mut cache = ResolutionContext::new(&env);
        let _ = properties_of_key(&mut cache, &a, 0);
    });
}

#[test]
#[should_panic]
fn test_depth_past_the_cap_is_rejected() {
    // Distinct from the cycle guard: this terminates, but a price path that
    // exhausts the CPU budget is a position that cannot be liquidated.
    let env = Env::default();
    let reflector = Address::generate(&env);
    with_contract(&env, || {
        let mut cache = ResolutionContext::new(&env);
        let source = PriceSource::Feed(twap_feed(&env, &reflector));
        let _ = properties_of_source(&mut cache, &source, common::types::MAX_RESOLUTION_DEPTH + 1);
    });
}

#[test]
fn test_config_properties_carry_the_one_or_two_arity() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    with_contract(&env, || {
        let mut cache = ResolutionContext::new(&env);

        let single = one(&env, PriceSource::Feed(twap_feed(&env, &reflector)));
        let derived = properties_of_config(&mut cache, &single);
        assert!(derived.second.is_none());

        let mut pair = single.clone();
        pair.push_back(PriceSource::Feed(nav_feed(&env, &adapter, "nav")));
        let derived = properties_of_config(&mut cache, &pair);
        assert!(derived.second.is_some());
        assert_eq!(
            derived.combined().trust.len(),
            2,
            "combined properties span both opinions"
        );
    });
}

// ---------------------------------------------------------------------------
// End to end: the config that motivated the redesign.
// ---------------------------------------------------------------------------

/// Builds the SolvBTC shape. Source 0 is the RedStone SolvBTC/BTC ratio scaled
/// by a Reflector BTC TWAP; source 1 is the RedStone direct SolvBTC/USD feed.
/// Both legs terminate at the same adapter, so the config has to say so.
fn solvbtc_oracle(
    env: &Env,
    adapter: &Address,
    btc: PriceKey,
    independence: IndependencePolicy,
) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor: nav_feed(env, adapter, "SolvBTC_FUNDAMENTAL"),
        quote: btc,
        // SolvBTC sits just above 1.0 BTC and only accrues upward.
        min_factor_wad: 10i128.pow(18),
        max_factor_wad: 2 * 10i128.pow(18),
    }));
    sources.push_back(PriceSource::Feed(nav_feed(
        env,
        adapter,
        "SolvBTC_FUNDAMENTAL/USD",
    )));

    AssetOracle {
        asset_decimals: 8,
        max_price_stale_seconds: 86_400,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        independence,
        min_sanity_price_wad: 20_000 * 10i128.pow(18),
        max_sanity_price_wad: 200_000 * 10i128.pow(18),
    }
}

fn register_btc_reference(env: &Env, reflector: &Address) -> PriceKey {
    let btc = PriceKey::Ref(Symbol::new(env, "BTC"));
    registry::set_oracle(
        env,
        &btc,
        &oracle_of(env, one(env, PriceSource::Feed(twap_feed(env, reflector)))),
    );
    btc
}

#[test]
fn test_solvbtc_config_validates_with_its_shared_adapter_declared() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let solvbtc = PriceKey::Token(Address::generate(&env));

    with_contract(&env, || {
        let btc = register_btc_reference(&env, &reflector);

        let mut declared = Vec::new(&env);
        declared.push_back(common::types::TrustDomain {
            kind: ProviderKind::RedStone,
            contract: adapter.clone(),
        });

        let oracle = solvbtc_oracle(
            &env,
            &adapter,
            btc,
            IndependencePolicy::AllowShared(declared),
        );
        crate::config::set_asset_oracle(&env, solvbtc.clone(), oracle);

        // The asset that no v1 configuration could express is now stored.
        let stored = registry::resolve_oracle(&env, &solvbtc).expect("solvbtc must be configured");
        assert!(stored.is_dual());
    });
}

#[test]
#[should_panic]
fn test_solvbtc_config_is_rejected_without_the_declaration() {
    // The shared adapter is real, so claiming disjoint independence must fail.
    // This is the disclosure the whole policy exists to force.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let solvbtc = PriceKey::Token(Address::generate(&env));

    with_contract(&env, || {
        let btc = register_btc_reference(&env, &reflector);
        let oracle = solvbtc_oracle(&env, &adapter, btc, IndependencePolicy::RequireDisjoint);
        crate::config::set_asset_oracle(&env, solvbtc, oracle);
    });
}

#[test]
#[should_panic]
fn test_a_ratio_leg_outliving_the_asset_ceiling_is_rejected() {
    // The C1 regression, as a config rule: a component permitted to sit frozen
    // longer than the asset's own answer may be stale would let a live quote
    // keep the composite looking fresh.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let solvbtc = PriceKey::Token(Address::generate(&env));

    with_contract(&env, || {
        let btc = register_btc_reference(&env, &reflector);
        let mut declared = Vec::new(&env);
        declared.push_back(common::types::TrustDomain {
            kind: ProviderKind::RedStone,
            contract: adapter.clone(),
        });
        let mut oracle = solvbtc_oracle(
            &env,
            &adapter,
            btc,
            IndependencePolicy::AllowShared(declared),
        );
        // Legs are 86400; drop the ceiling under them.
        oracle.max_price_stale_seconds = 3_600;
        crate::config::set_asset_oracle(&env, solvbtc, oracle);
    });
}
