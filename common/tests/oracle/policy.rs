use super::*;
use crate::types::oracle::{OracleAssetRef, OracleReadMode};
use crate::types::oracle_v2::{
    FeedNature, FeedSource, MultiFeedRef, PriceKey, ProviderKind, ProviderRef, ReflectorFeedRef,
    TrustDomain,
};
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

// ---------------------------------------------------------------------------
// Source count
// ---------------------------------------------------------------------------

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
    // Three sources would silently become a median rule, which hides a bad
    // source instead of failing closed on it.
    let env = Env::default();
    validate_source_count(&env, 3);
}

// ---------------------------------------------------------------------------
// Depth
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Staleness envelope
// ---------------------------------------------------------------------------

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
    // The regression this exists to prevent: a slow leg permitted to sit frozen
    // longer than the asset's own answer is allowed to be stale, so a live fast
    // leg keeps the composite looking fresh.
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

// ---------------------------------------------------------------------------
// Smoothing
// ---------------------------------------------------------------------------

#[test]
fn test_smoothing_accepts_single_fundamental_source() {
    // Every Hub 2 RWA market is one push-oracle NAV feed. v1 permitted this only
    // because the spot rule was scoped to anchored markets; here it passes for
    // the actual reason - trading cannot move a published NAV.
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

// ---------------------------------------------------------------------------
// Independence
// ---------------------------------------------------------------------------

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
    // Source 0: Reflector BTC TWAP joined with the RedStone ratio feed.
    // Source 1: the RedStone direct feed on the same adapter.
    // They share exactly one domain, and the config must say so.
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
    // Set equality, not subset: a stale waiver naming a domain the config no
    // longer shares must not silently keep passing.
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
    // Different addresses share no contract, so nothing is waived and nothing
    // is rejected. An earlier version of this rule rejected the shape outright
    // via a provider-kind floor - and offered no waiver, because a *computed*
    // shared set that is empty can never be matched by a declaration. That
    // reproduced exactly the over-strictness this model was meant to remove.
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
    // The forgery the address-level rule exists to stop: `kind` on a multi-feed
    // adapter is declared by the proposer and unverifiable on-chain, so two
    // feeds on ONE contract can be labelled RedStone and Xoxno and would read as
    // disjoint if sharing were judged on the (kind, contract) pair.
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
    // `AllowShared([])` is `RequireDisjoint` spelled differently; two ways to
    // say the same thing defeats any off-chain rule keyed on the variant.
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

// ---------------------------------------------------------------------------
// Factor bounds
// ---------------------------------------------------------------------------

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
fn test_factor_in_bounds_accepts_the_inclusive_edges() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    let scaled = scaled_with_bounds(&env, wad, 2 * wad);
    require_factor_in_bounds(&env, wad, &scaled);
    require_factor_in_bounds(&env, 2 * wad, &scaled);
}

#[test]
#[should_panic]
fn test_factor_below_bounds_reverts() {
    // A compromised ratio feed reporting 0.5 instead of 1.003 would otherwise
    // halve the asset's price inside an output band sized for BTC volatility.
    let env = Env::default();
    let wad = 10i128.pow(18);
    let scaled = scaled_with_bounds(&env, wad, 2 * wad);
    require_factor_in_bounds(&env, wad / 2, &scaled);
}

#[test]
#[should_panic]
fn test_factor_above_bounds_reverts() {
    let env = Env::default();
    let wad = 10i128.pow(18);
    let scaled = scaled_with_bounds(&env, wad, 2 * wad);
    require_factor_in_bounds(&env, 3 * wad, &scaled);
}
