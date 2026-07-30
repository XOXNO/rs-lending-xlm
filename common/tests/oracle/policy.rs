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
fn test_staleness_envelope_accepts_component_exactly_at_ceiling() {
    // The bound is "may not OUTLIVE the ceiling", so equality is legal. Without
    // this case nothing distinguishes `>` from `>=`, and `>=` would reject every
    // config that sizes a single leg's window to the asset ceiling — the normal
    // shape for a single-source asset.
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

/// One adapter carrying two labels on the left, the same adapter on the right.
/// Both dedup paths in the independence rule collapse a contract that appears
/// more than once, and the counts they produce are compared against each other —
/// so a dedup that stops deduping turns a correct waiver into a rejection.
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
    // The left source names the adapter twice (once per label), so the shared
    // set would hold it twice without dedup — and would then no longer match a
    // waiver that correctly names it once.
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
    // Mirror of the case above, on the declaration side: a proposer waiving both
    // labels of one adapter has waived one contract, and that must still match a
    // shared set holding it once.
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
fn test_factor_bounds_accept_a_degenerate_pinned_range() {
    // min == max pins the factor to one value. That is a legal, maximally tight
    // band, not an inverted one — the rejection is `max < min`. Only this case
    // separates `<` from `<=`.
    let env = Env::default();
    let wad = 10i128.pow(18);
    validate_factor_bounds(&env, &scaled_with_bounds(&env, wad, wad));
}

#[test]
#[should_panic]
fn test_factor_bounds_reject_max_above_reasonable_price_cap() {
    let env = Env::default();
    // Same economic ceiling as USD sanity: oversized max would only enable
    // overflow-shaped configs without adding real pricing headroom.
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

// ---------------------------------------------------------------------------
// Feed shape
// ---------------------------------------------------------------------------
//
// Every bound below is inclusive, so each needs a pass-at-the-edge case as well
// as a reject-past-it case. An edge case alone cannot tell `<` from `<=`.

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
    // Past WAD the rescale factor overflows and traps as a raw wasm error
    // instead of a typed one.
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
    // A one-second leg makes the feed permanently stale and the market
    // unusable. The envelope check cannot catch this: it only proves a leg does
    // not outlive the asset ceiling.
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
    // MIN_SMOOTHING_TWAP_RECORDS samples is the fewest that count as smoothing.
    // Every other TWAP case in this file uses 3, which leaves the boundary
    // itself unpinned.
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
    // A one-sample average is a spot read wearing a different label: it would
    // satisfy `validate_smoothing`, whose whole justification is that moving a
    // time-average costs more than moving one print.
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
    // Distinct from the one-sample case: `Twap(0)` reads as smoothed, validates,
    // then reverts on every read — a market born bricked.
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
    // `kind` names the operator behind a multi-feed adapter, and it decides how
    // that adapter's decimals are established. A Reflector deployment is
    // addressed by asset through ProviderRef::Reflector, never by feed id, so
    // the label names no multi-feed operator and has no defined attestation.
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

// ---------------------------------------------------------------------------
// Source shape
// ---------------------------------------------------------------------------

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
    // Scaled delegates to BOTH validate_feed_shape (on the factor leg) and
    // validate_factor_bounds; a delegation dropped here would leave a malformed
    // factor leg configurable.
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
#[should_panic]
fn test_source_shape_refuses_lp_shares_unconditionally() {
    // Not reachable through the smoothing rule alone — an LP source paired with
    // any clean source satisfies "at least one opinion is clean", so the config
    // would store and then revert on every read. Refused here instead, where the
    // refusal does not depend on the pairing.
    let env = Env::default();
    validate_source_shape(
        &env,
        &PriceSource::LpShare(LpShareSource {
            pool: Address::generate(&env),
            kind: PoolKind::ConstantProduct,
            key_a: PriceKey::Ref(Symbol::new(&env, "BTC")),
            key_b: PriceKey::Ref(Symbol::new(&env, "USD")),
            reserve_a_decimals: 8,
            reserve_b_decimals: 7,
            share_decimals: 7,
        }),
    );
}

// ---------------------------------------------------------------------------
// Asset decimals
// ---------------------------------------------------------------------------
//
// These scale every token amount a consumer derives from the price, including
// liquidation seize amounts and protocol fees.

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
    // A reference price has no token and no amounts, so it carries zero and
    // nothing else — including nothing from the token range.
    let env = Env::default();
    validate_asset_decimals(&env, &PriceKey::Ref(Symbol::new(&env, "BTC")), 0);
}

#[test]
#[should_panic]
fn test_asset_decimals_reject_a_nonzero_reference_key() {
    let env = Env::default();
    validate_asset_decimals(&env, &PriceKey::Ref(Symbol::new(&env, "BTC")), 8);
}
