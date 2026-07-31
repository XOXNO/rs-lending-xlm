use super::*;
use soroban_sdk::{testutils::Address as _, Env, Symbol};

fn reflector_twap(env: &Env, contract: &Address, records: u32) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Twap(records),
        }),
        decimals: 14,
        max_stale_seconds: 3600,
    }
}

fn reflector_spot(env: &Env, contract: &Address) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: OracleReadMode::Spot,
        }),
        decimals: 14,
        max_stale_seconds: 3600,
    }
}

fn multi_feed(env: &Env, contract: &Address, feed: &str, kind: ProviderKind) -> FeedSource {
    multi_feed_of(env, contract, feed, kind, FeedNature::Fundamental)
}

fn multi_feed_of(
    env: &Env,
    contract: &Address,
    feed: &str,
    kind: ProviderKind,
    nature: FeedNature,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: contract.clone(),
            feed_id: String::from_str(env, feed),
            kind,
            nature,
        }),
        decimals: 8,
        max_stale_seconds: 43200,
    }
}

#[test]
fn test_twap_reflector_is_smoothed_spot_is_not() {
    let env = Env::default();
    let contract = Address::generate(&env);
    assert!(reflector_twap(&env, &contract, 3).provider.is_smoothed());
    assert!(!reflector_spot(&env, &contract).provider.is_smoothed());
}

#[test]
fn test_multi_feed_is_not_smoothed() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let feed = multi_feed(
        &env,
        &contract,
        "SolvBTC_FUNDAMENTAL",
        ProviderKind::RedStone,
    );
    assert!(!feed.provider.is_smoothed());
}

#[test]
fn test_redstone_and_xoxno_are_distinct_domains_at_one_address() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let redstone = multi_feed(&env, &contract, "XLM", ProviderKind::RedStone);
    let xoxno = multi_feed(&env, &contract, "XLM", ProviderKind::Xoxno);
    assert_ne!(
        TrustDomain::of(&redstone.provider),
        TrustDomain::of(&xoxno.provider),
    );
}

#[test]
fn test_two_feeds_on_one_adapter_share_a_domain() {
    let env = Env::default();
    let contract = Address::generate(&env);

    let a = multi_feed(
        &env,
        &contract,
        "SolvBTC_FUNDAMENTAL",
        ProviderKind::RedStone,
    );
    let b = multi_feed(
        &env,
        &contract,
        "SolvBTC_FUNDAMENTAL/USD",
        ProviderKind::RedStone,
    );
    assert_eq!(TrustDomain::of(&a.provider), TrustDomain::of(&b.provider));

    let pa = SourceProperties::of_feed(&env, &a);
    let pb = SourceProperties::of_feed(&env, &b);
    assert_eq!(pa.shared_contracts_with(&env, &pb).len(), 1);
}

#[test]
fn test_join_propagates_defect_unions_trust_and_takes_max_depth() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    let smoothed = SourceProperties::of_feed(&env, &reflector_twap(&env, &reflector, 3));
    let unsmoothed = SourceProperties::of_feed(
        &env,
        &multi_feed(&env, &adapter, "X", ProviderKind::RedStone),
    )
    .nest();

    let joined = smoothed.join(&unsmoothed);
    assert!(
        !joined.has_unsmoothed_market_leg,
        "a fundamental leg carries no smoothing defect to propagate"
    );
    assert_eq!(joined.trust.len(), 2);
    assert_eq!(joined.depth, 1, "join takes the deeper branch");
}

#[test]
fn test_join_takes_the_deeper_branch_in_either_order() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let base = SourceProperties::of_feed(&env, &reflector_twap(&env, &reflector, 3));

    let shallow = base.clone().nest();
    let deep = base.clone().nest().nest();
    assert_eq!(shallow.depth, 1);
    assert_eq!(deep.depth, 2);

    assert_eq!(deep.join(&shallow).depth, 2, "deeper self wins");
    assert_eq!(shallow.join(&deep).depth, 2, "deeper other wins");
    assert_eq!(
        shallow.join(&shallow).depth,
        1,
        "equal depths compose to the same level"
    );
}

#[test]
fn test_join_deduplicates_trust() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let a = SourceProperties::of_feed(
        &env,
        &multi_feed(&env, &adapter, "A", ProviderKind::RedStone),
    );
    let b = SourceProperties::of_feed(
        &env,
        &multi_feed(&env, &adapter, "B", ProviderKind::RedStone),
    );
    assert_eq!(a.join(&b).trust.len(), 1);
}

#[test]
fn test_join_is_symmetric_in_smoothing_and_trust() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let a = SourceProperties::of_feed(&env, &reflector_twap(&env, &reflector, 3));
    let b = SourceProperties::of_feed(&env, &multi_feed(&env, &adapter, "X", ProviderKind::Xoxno));

    let ab = a.join(&b);
    let ba = b.join(&a);
    assert_eq!(ab.has_unsmoothed_market_leg, ba.has_unsmoothed_market_leg);
    assert_eq!(ab.depth, ba.depth);
    assert_eq!(ab.trust.len(), ba.trust.len());
    assert!(ab.trust.iter().all(|d| contains_domain(&ba.trust, &d)));
}

#[test]
fn test_feed_source_has_no_dependencies() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let source = PriceSource::Feed(reflector_twap(&env, &contract, 3));
    let props = local_properties(&env, &source);
    assert_eq!(props.dependencies.len(), 0);
    assert_eq!(props.local.depth, 0);
    assert!(!props.local.has_unsmoothed_market_leg);
}

#[test]
fn test_scaled_source_depends_on_its_quote_only() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let quote = PriceKey::Ref(Symbol::new(&env, "BTC"));
    let source = PriceSource::Scaled(ScaledSource {
        factor: multi_feed(
            &env,
            &adapter,
            "SolvBTC_FUNDAMENTAL",
            ProviderKind::RedStone,
        ),
        quote: quote.clone(),
        min_factor_wad: 900_000_000_000_000_000,
        max_factor_wad: 1_500_000_000_000_000_000,
    });

    let props = local_properties(&env, &source);
    assert_eq!(props.dependencies.len(), 1);
    assert_eq!(props.dependencies.get(0).unwrap(), quote);

    assert_eq!(props.local.trust.len(), 1);
    assert!(
        !props.local.has_unsmoothed_market_leg,
        "a fundamental ratio is not moved by trading, so it needs no window"
    );
}

#[test]
fn test_lp_source_contributes_no_provider_and_two_dependencies() {
    let env = Env::default();
    let pool = Address::generate(&env);
    let token_a = PriceKey::Token(Address::generate(&env));
    let token_b = PriceKey::Ref(Symbol::new(&env, "BTC"));

    let source = PriceSource::LpShare(LpShareSource {
        pool,
        plane: Address::generate(&env),
        kind: PoolKind::ConstantProduct,
        key_a: token_a.clone(),
        key_b: token_b.clone(),
        reserve_a_decimals: 7,
        reserve_b_decimals: 8,
        share_decimals: 7,
    });

    let props = local_properties(&env, &source);

    assert_eq!(props.local.trust.len(), 0);
    assert!(
        props.local.has_unsmoothed_market_leg,
        "an LP reserve read is market state with no window at any price"
    );
    assert_eq!(props.dependencies.len(), 2);
    assert_eq!(props.dependencies.get(0).unwrap(), token_a);
    assert_eq!(props.dependencies.get(1).unwrap(), token_b);
}

#[test]
fn test_is_dual_tracks_source_count() {
    let env = Env::default();
    let contract = Address::generate(&env);

    let mut sources = Vec::new(&env);
    sources.push_back(PriceSource::Feed(reflector_twap(&env, &contract, 3)));

    let mut oracle = AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 3600,
        sources: sources.clone(),
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX / 2,
    };
    assert!(!oracle.is_dual());

    sources.push_back(PriceSource::Feed(multi_feed(
        &env,
        &Address::generate(&env),
        "X",
        ProviderKind::RedStone,
    )));
    oracle.sources = sources;
    assert!(oracle.is_dual());
}

#[test]
fn test_unsmoothed_market_leg_is_the_defect_smoothing_guards() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    assert!(reflector_spot(&env, &reflector)
        .provider
        .is_unsmoothed_market_leg());

    assert!(!reflector_twap(&env, &reflector, 3)
        .provider
        .is_unsmoothed_market_leg());

    let market_push = multi_feed_of(
        &env,
        &adapter,
        "BTC",
        ProviderKind::RedStone,
        FeedNature::Market,
    );
    assert!(market_push.provider.is_unsmoothed_market_leg());

    let nav = multi_feed_of(
        &env,
        &adapter,
        "SolvBTC_FUNDAMENTAL",
        ProviderKind::RedStone,
        FeedNature::Fundamental,
    );
    assert!(!nav.provider.is_unsmoothed_market_leg());
}

#[test]
fn test_defect_taints_the_whole_source_through_join() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    let clean = SourceProperties::of_feed(&env, &reflector_twap(&env, &reflector, 3));
    let defective = SourceProperties::of_feed(
        &env,
        &multi_feed_of(
            &env,
            &adapter,
            "BTC",
            ProviderKind::RedStone,
            FeedNature::Market,
        ),
    );

    assert!(clean.join(&defective).has_unsmoothed_market_leg);
    assert!(defective.join(&clean).has_unsmoothed_market_leg);
}

#[test]
fn test_join_reports_the_loosest_staleness_bound() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);

    let fast = SourceProperties::of_feed(&env, &reflector_twap(&env, &reflector, 3));
    let slow = SourceProperties::of_feed(
        &env,
        &multi_feed(&env, &adapter, "X", ProviderKind::RedStone),
    );

    assert_eq!(fast.join(&slow).loosest_max_stale_seconds, 43200);
    assert_eq!(slow.join(&fast).loosest_max_stale_seconds, 43200);
}

#[test]
fn test_empty_properties_do_not_loosen_freshness() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let feed = SourceProperties::of_feed(
        &env,
        &multi_feed(&env, &adapter, "X", ProviderKind::RedStone),
    );

    let joined = SourceProperties::empty(&env).join(&feed);
    assert_eq!(joined.loosest_max_stale_seconds, 43200);
}
