use super::*;
use crate::constants::MAX_REASONABLE_PRICE_WAD;
use crate::types::composable_oracle::{
    FeedNature, FeedSource, LpShareSource, MultiFeedRef, PoolKind, PriceKey, ProviderKind,
    ProviderRef, ReflectorFeedRef, TrustDomain,
};
use crate::types::oracle::{OracleAssetRef, OracleReadMode};
use soroban_sdk::{testutils::Address as _, Address, String, Symbol};

fn reflector(env: &Env, contract: &Address, mode: OracleReadMode, max_stale: u64) -> FeedSource {
    FeedSource {
        provider: ProviderRef::Reflector(ReflectorFeedRef {
            contract: contract.clone(),
            asset: OracleAssetRef::Symbol(Symbol::new(env, "BTC")),
            read_mode: mode,
        }),
        decimals: 14,
        max_stale_seconds: max_stale,
    }
}

fn adapter_feed(
    env: &Env,
    contract: &Address,
    feed: &str,
    kind: ProviderKind,
    nature: FeedNature,
    max_stale: u64,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: contract.clone(),
            feed_id: String::from_str(env, feed),
            kind,
            nature,
        }),
        decimals: 8,
        max_stale_seconds: max_stale,
    }
}

fn props(env: &Env, feed: &FeedSource) -> SourceProperties {
    SourceProperties::of_feed(env, feed)
}

#[test]
fn test_source_count_accepts_one_and_two() {
    let env = Env::default();
    validate_source_count(&env, 1);
    validate_source_count(&env, 2);
}

#[test]
#[should_panic]
fn test_source_count_rejects_zero() {
    let env = Env::default();
    validate_source_count(&env, 0);
}

#[test]
#[should_panic]
fn test_source_count_rejects_three() {
    let env = Env::default();
    validate_source_count(&env, 3);
}

#[test]
fn test_depth_accepts_at_the_cap() {
    let env = Env::default();
    let mut p = SourceProperties::empty(&env);
    p.depth = MAX_RESOLUTION_DEPTH;
    validate_composition_depth(&env, &p);
}

#[test]
#[should_panic]
fn test_depth_rejects_past_the_cap() {
    let env = Env::default();
    let mut p = SourceProperties::empty(&env);
    p.depth = MAX_RESOLUTION_DEPTH + 1;
    validate_composition_depth(&env, &p);
}

#[test]
fn test_staleness_envelope_accepts_component_under_ceiling() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let p = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "SolvBTC_FUNDAMENTAL",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_staleness_envelope(&env, 86_400, &p);
}

#[test]
#[should_panic]
fn test_staleness_envelope_rejects_component_outliving_ceiling() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let p = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "ratio",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            86_400,
        ),
    );
    validate_staleness_envelope(&env, 3_600, &p);
}

#[test]
fn test_staleness_envelope_accepts_component_exactly_at_ceiling() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let p = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "SolvBTC_FUNDAMENTAL",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_staleness_envelope(&env, 43_200, &p);
}

#[test]
#[should_panic]
fn test_staleness_envelope_rejects_ceiling_past_protocol_max() {
    let env = Env::default();
    let p = SourceProperties::empty(&env);
    validate_staleness_envelope(&env, MAX_PRICE_STALE_SECONDS + 1, &p);
}

#[test]
#[should_panic]
fn test_staleness_envelope_rejects_ceiling_below_protocol_min() {
    let env = Env::default();
    let p = SourceProperties::empty(&env);
    validate_staleness_envelope(&env, MIN_PRICE_STALE_SECONDS - 1, &p);
}

#[test]
fn test_smoothing_accepts_single_fundamental_source() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let nav = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "USST_FUNDAMENTAL",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_smoothing(&env, &nav, None);
}

#[test]
#[should_panic]
fn test_smoothing_rejects_single_unsmoothed_market_source() {
    let env = Env::default();
    let contract = Address::generate(&env);
    let spot = props(
        &env,
        &reflector(&env, &contract, OracleReadMode::Spot, 3_600),
    );
    validate_smoothing(&env, &spot, None);
}

#[test]
fn test_smoothing_accepts_dual_when_one_opinion_is_clean() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let dirty = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Spot, 3_600),
    );
    let clean = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "nav",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_smoothing(&env, &dirty, Some(&clean));
}

#[test]
#[should_panic]
fn test_smoothing_rejects_dual_when_both_opinions_are_movable() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let first = props(&env, &reflector(&env, &a, OracleReadMode::Spot, 3_600));
    let second = props(&env, &reflector(&env, &b, OracleReadMode::Spot, 3_600));
    validate_smoothing(&env, &first, Some(&second));
}

#[test]
fn test_smoothing_is_order_independent() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let dirty = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Spot, 3_600),
    );
    let clean = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "nav",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_smoothing(&env, &dirty, Some(&clean));
    validate_smoothing(&env, &clean, Some(&dirty));
}

#[test]
fn test_disjoint_sources_pass_require_disjoint() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Twap(3), 3_600),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "x",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(&env, &a, &b, &IndependencePolicy::RequireDisjoint);
}

#[test]
#[should_panic]
fn test_shared_adapter_fails_require_disjoint() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "ratio",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "direct",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(&env, &a, &b, &IndependencePolicy::RequireDisjoint);
}

#[test]
fn test_solvbtc_shape_passes_only_with_an_exact_declaration() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);

    let ratio = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "SolvBTC_FUNDAMENTAL",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            86_400,
        ),
    );
    let quote = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Twap(3), 3_600),
    );
    let scaled = ratio.join(&quote).nest();

    let direct = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "SolvBTC_FUNDAMENTAL/USD",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );

    let mut declared = Vec::new(&env);
    declared.push_back(TrustDomain {
        kind: ProviderKind::RedStone,
        contract: adapter.clone(),
    });
    validate_independence(
        &env,
        &scaled,
        &direct,
        &IndependencePolicy::AllowShared(declared),
    );
}

#[test]
#[should_panic]
fn test_empty_declaration_does_not_waive_a_real_shared_domain() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "a",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "b",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(
        &env,
        &a,
        &b,
        &IndependencePolicy::AllowShared(Vec::new(&env)),
    );
}

#[test]
#[should_panic]
fn test_declaring_a_domain_that_is_not_shared_is_rejected() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Twap(3), 3_600),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "x",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    let mut declared = Vec::new(&env);
    declared.push_back(TrustDomain {
        kind: ProviderKind::RedStone,
        contract: adapter,
    });
    validate_independence(&env, &a, &b, &IndependencePolicy::AllowShared(declared));
}

#[test]
fn test_two_deployments_of_one_provider_are_allowed() {
    let env = Env::default();
    let cex = Address::generate(&env);
    let dex = Address::generate(&env);
    let a = props(&env, &reflector(&env, &cex, OracleReadMode::Twap(3), 3_600));
    let b = props(&env, &reflector(&env, &dex, OracleReadMode::Twap(3), 3_600));
    validate_independence(&env, &a, &b, &IndependencePolicy::RequireDisjoint);
}

#[test]
#[should_panic]
fn test_one_adapter_relabelled_as_two_providers_is_still_shared() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "A",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "B",
            ProviderKind::Xoxno,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(&env, &a, &b, &IndependencePolicy::RequireDisjoint);
}

#[test]
#[should_panic]
fn test_an_empty_waiver_is_rejected() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Twap(3), 3_600),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "x",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(
        &env,
        &a,
        &b,
        &IndependencePolicy::AllowShared(Vec::new(&env)),
    );
}

#[test]
fn test_independence_is_order_independent() {
    let env = Env::default();
    let reflector_contract = Address::generate(&env);
    let adapter = Address::generate(&env);
    let a = props(
        &env,
        &reflector(&env, &reflector_contract, OracleReadMode::Twap(3), 3_600),
    );
    let b = props(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "x",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    validate_independence(&env, &a, &b, &IndependencePolicy::RequireDisjoint);
    validate_independence(&env, &b, &a, &IndependencePolicy::RequireDisjoint);
}

fn one_adapter_two_labels(env: &Env, adapter: &Address) -> (SourceProperties, SourceProperties) {
    let relabelled = props(
        env,
        &adapter_feed(
            env,
            adapter,
            "A",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    )
    .join(&props(
        env,
        &adapter_feed(
            env,
            adapter,
            "B",
            ProviderKind::Xoxno,
            FeedNature::Fundamental,
            43_200,
        ),
    ));
    let single = props(
        env,
        &adapter_feed(
            env,
            adapter,
            "C",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            43_200,
        ),
    );
    (relabelled, single)
}

#[test]
fn test_a_contract_shared_under_two_labels_counts_once() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let (relabelled, single) = one_adapter_two_labels(&env, &adapter);
    assert_eq!(
        relabelled.trust.len(),
        2,
        "two labels on one contract are two trust domains"
    );
    assert_eq!(
        relabelled.shared_contracts_with(&env, &single).len(),
        1,
        "but only one shared contract"
    );

    validate_independence(
        &env,
        &relabelled,
        &single,
        &IndependencePolicy::AllowShared(soroban_sdk::vec![
            &env,
            TrustDomain {
                kind: ProviderKind::RedStone,
                contract: adapter.clone(),
            }
        ]),
    );
}

#[test]
fn test_a_waiver_may_name_one_contract_under_two_labels() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let (relabelled, single) = one_adapter_two_labels(&env, &adapter);

    validate_independence(
        &env,
        &relabelled,
        &single,
        &IndependencePolicy::AllowShared(soroban_sdk::vec![
            &env,
            TrustDomain {
                kind: ProviderKind::RedStone,
                contract: adapter.clone(),
            },
            TrustDomain {
                kind: ProviderKind::Xoxno,
                contract: adapter.clone(),
            }
        ]),
    );
}

fn scaled_with_bounds(env: &Env, min_wad: i128, max_wad: i128) -> ScaledSource {
    let adapter = Address::generate(env);
    ScaledSource {
        factor: adapter_feed(
            env,
            &adapter,
            "SolvBTC_FUNDAMENTAL",
            ProviderKind::RedStone,
            FeedNature::Fundamental,
            86_400,
        ),
        quote: PriceKey::Ref(Symbol::new(env, "BTC")),
        min_factor_wad: min_wad,
        max_factor_wad: max_wad,
    }
}

#[test]
fn test_factor_bounds_accept_a_positive_range() {
    let env = Env::default();
    validate_factor_bounds(
        &env,
        &scaled_with_bounds(&env, 10i128.pow(18), 2 * 10i128.pow(18)),
    );
}

#[test]
#[should_panic]
fn test_factor_bounds_reject_non_positive_minimum() {
    let env = Env::default();
    validate_factor_bounds(&env, &scaled_with_bounds(&env, 0, 10i128.pow(18)));
}

#[test]
#[should_panic]
fn test_factor_bounds_reject_inverted_range() {
    let env = Env::default();
    validate_factor_bounds(
        &env,
        &scaled_with_bounds(&env, 2 * 10i128.pow(18), 10i128.pow(18)),
    );
}

#[test]
fn test_factor_bounds_accept_a_degenerate_pinned_range() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    validate_factor_bounds(&env, &scaled_with_bounds(&env, wad, wad));
}

#[test]
#[should_panic]
fn test_factor_bounds_reject_max_above_reasonable_price_cap() {
    let env = Env::default();

    validate_factor_bounds(
        &env,
        &scaled_with_bounds(&env, 10i128.pow(18), MAX_REASONABLE_PRICE_WAD + 1),
    );
}

#[test]
fn test_factor_bounds_accept_max_at_reasonable_price_cap() {
    let env = Env::default();
    validate_factor_bounds(
        &env,
        &scaled_with_bounds(&env, 10i128.pow(18), MAX_REASONABLE_PRICE_WAD),
    );
}

fn spot_feed(env: &Env, decimals: u32, max_stale: u64) -> FeedSource {
    let mut feed = reflector(
        env,
        &Address::generate(env),
        OracleReadMode::Spot,
        max_stale,
    );
    feed.decimals = decimals;
    feed
}

#[test]
fn test_feed_shape_accepts_the_inclusive_decimal_edges() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, MIN_ORACLE_DECIMALS, 3_600));
    validate_feed_shape(&env, &spot_feed(&env, MAX_ORACLE_DECIMALS, 3_600));
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_decimals_below_the_floor() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, MIN_ORACLE_DECIMALS - 1, 3_600));
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_decimals_past_the_wad_scale() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, MAX_ORACLE_DECIMALS + 1, 3_600));
}

#[test]
fn test_feed_shape_accepts_the_inclusive_staleness_edges() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, 14, MIN_PRICE_STALE_SECONDS));
    validate_feed_shape(&env, &spot_feed(&env, 14, MAX_PRICE_STALE_SECONDS));
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_a_leg_window_below_the_floor() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, 14, MIN_PRICE_STALE_SECONDS - 1));
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_a_leg_window_past_the_protocol_max() {
    let env = Env::default();
    validate_feed_shape(&env, &spot_feed(&env, 14, MAX_PRICE_STALE_SECONDS + 1));
}

#[test]
fn test_feed_shape_accepts_twap_at_the_smoothing_floor() {
    let env = Env::default();
    let contract = Address::generate(&env);
    validate_feed_shape(
        &env,
        &reflector(
            &env,
            &contract,
            OracleReadMode::Twap(MIN_SMOOTHING_TWAP_RECORDS),
            3_600,
        ),
    );
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_a_one_sample_twap() {
    let env = Env::default();
    let contract = Address::generate(&env);
    validate_feed_shape(
        &env,
        &reflector(
            &env,
            &contract,
            OracleReadMode::Twap(MIN_SMOOTHING_TWAP_RECORDS - 1),
            3_600,
        ),
    );
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_a_zero_sample_twap() {
    let env = Env::default();
    let contract = Address::generate(&env);
    validate_feed_shape(
        &env,
        &reflector(&env, &contract, OracleReadMode::Twap(0), 3_600),
    );
}

#[test]
#[should_panic]
fn test_feed_shape_rejects_a_multi_feed_labelled_reflector() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    validate_feed_shape(
        &env,
        &adapter_feed(
            &env,
            &adapter,
            "BTC",
            ProviderKind::Reflector,
            FeedNature::Market,
            3_600,
        ),
    );
}

#[test]
fn test_source_shape_validates_a_bare_feed() {
    let env = Env::default();
    validate_source_shape(&env, &PriceSource::Feed(spot_feed(&env, 14, 3_600)));
}

#[test]
#[should_panic]
fn test_source_shape_validates_the_feed_inside_a_bare_feed() {
    let env = Env::default();
    validate_source_shape(
        &env,
        &PriceSource::Feed(spot_feed(&env, MAX_ORACLE_DECIMALS + 1, 3_600)),
    );
}

#[test]
fn test_source_shape_validates_a_scaled_source() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    validate_source_shape(
        &env,
        &PriceSource::Scaled(scaled_with_bounds(&env, wad, 2 * wad)),
    );
}

#[test]
#[should_panic]
fn test_source_shape_validates_the_factor_feed_of_a_scaled_source() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    let mut scaled = scaled_with_bounds(&env, wad, 2 * wad);
    scaled.factor.max_stale_seconds = MAX_PRICE_STALE_SECONDS + 1;
    validate_source_shape(&env, &PriceSource::Scaled(scaled));
}

#[test]
#[should_panic]
fn test_source_shape_validates_the_bounds_of_a_scaled_source() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    validate_source_shape(
        &env,
        &PriceSource::Scaled(scaled_with_bounds(&env, 2 * wad, wad)),
    );
}

#[test]
fn test_source_shape_accepts_constant_product_lp() {
    let env = Env::default();
    validate_source_shape(
        &env,
        &PriceSource::LpShare(LpShareSource {
            pool: Address::generate(&env),
            plane: Address::generate(&env),
            kind: PoolKind::ConstantProduct,
            key_a: PriceKey::Ref(Symbol::new(&env, "BTC")),
            key_b: PriceKey::Ref(Symbol::new(&env, "USD")),
            reserve_a_decimals: 8,
            reserve_b_decimals: 7,
            share_decimals: 7,
        }),
    );
}

#[test]
#[should_panic]
fn test_source_shape_refuses_lp_with_duplicate_legs() {
    let env = Env::default();
    let dup = PriceKey::Ref(Symbol::new(&env, "BTC"));
    validate_source_shape(
        &env,
        &PriceSource::LpShare(LpShareSource {
            pool: Address::generate(&env),
            plane: Address::generate(&env),
            kind: PoolKind::ConstantProduct,
            key_a: dup.clone(),
            key_b: dup,
            reserve_a_decimals: 8,
            reserve_b_decimals: 7,
            share_decimals: 7,
        }),
    );
}

#[test]
fn test_asset_decimals_accept_the_inclusive_token_edges() {
    let env = Env::default();
    let key = PriceKey::Token(Address::generate(&env));
    validate_asset_decimals(&env, &key, MIN_ASSET_DECIMALS);
    validate_asset_decimals(&env, &key, MAX_ASSET_DECIMALS);
}

#[test]
#[should_panic]
fn test_asset_decimals_reject_a_token_below_the_floor() {
    let env = Env::default();
    let key = PriceKey::Token(Address::generate(&env));
    validate_asset_decimals(&env, &key, MIN_ASSET_DECIMALS - 1);
}

#[test]
#[should_panic]
fn test_asset_decimals_reject_a_token_past_the_ceiling() {
    let env = Env::default();
    let key = PriceKey::Token(Address::generate(&env));
    validate_asset_decimals(&env, &key, MAX_ASSET_DECIMALS + 1);
}

#[test]
fn test_asset_decimals_accept_zero_for_a_reference_key() {
    let env = Env::default();
    validate_asset_decimals(&env, &PriceKey::Ref(Symbol::new(&env, "BTC")), 0);
}

#[test]
#[should_panic]
fn test_asset_decimals_reject_a_nonzero_reference_key() {
    let env = Env::default();
    validate_asset_decimals(&env, &PriceKey::Ref(Symbol::new(&env, "BTC")), 8);
}
