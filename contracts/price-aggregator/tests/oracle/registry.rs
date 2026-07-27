use super::*;
use common::types::{
    FeedNature, IndependencePolicy, OracleAssetRef, OracleReadMode, OracleSourceConfigOption,
    OracleTolerance, ProviderKind, RedStoneSourceConfig, ReflectorSourceConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{String, Symbol};

use crate::PriceAggregator;

const ASSET_STALE: u64 = 3_600;
const FEED_STALE: u64 = 43_200;

fn reflector_source(env: &Env, contract: &Address, base: ReflectorBase) -> OracleSourceConfig {
    OracleSourceConfig::Reflector(ReflectorSourceConfig {
        contract: contract.clone(),
        asset: OracleAssetRef::Symbol(Symbol::new(env, "XLM")),
        read_mode: OracleReadMode::Twap(3),
        decimals: 14,
        resolution_seconds: 300,
        base,
    })
}

fn redstone_source(env: &Env, contract: &Address, feed: &str) -> OracleSourceConfig {
    OracleSourceConfig::RedStone(RedStoneSourceConfig {
        contract: contract.clone(),
        feed_id: String::from_str(env, feed),
        decimals: 8,
        max_stale_seconds: FEED_STALE,
    })
}

fn legacy_config(
    primary: OracleSourceConfig,
    anchor: OracleSourceConfigOption,
    strategy: OracleStrategy,
) -> AssetOracleConfig {
    AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: ASSET_STALE,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        strategy,
        primary,
        anchor,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: 1_000_000_000_000_000_000,
    }
}

fn single_redstone(env: &Env, adapter: &Address) -> AssetOracleConfig {
    legacy_config(
        redstone_source(env, adapter, "USST_FUNDAMENTAL"),
        OracleSourceConfigOption::None,
        OracleStrategy::Single,
    )
}

fn with_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

// ---------------------------------------------------------------------------
// Lifting preserves behaviour, not field layout.
// ---------------------------------------------------------------------------

#[test]
fn test_single_strategy_lifts_to_one_source() {
    let env = Env::default();
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    let lifted = lift_legacy(&env, &config);
    assert_eq!(lifted.sources.len(), 1);
    assert!(!lifted.is_dual());
    assert_eq!(lifted.asset_decimals, 7);
    assert_eq!(lifted.max_price_stale_seconds, ASSET_STALE);
    assert_eq!(lifted.min_sanity_price_wad, config.min_sanity_price_wad);
    assert_eq!(lifted.max_sanity_price_wad, config.max_sanity_price_wad);
}

#[test]
fn test_anchored_strategy_lifts_to_two_sources_in_order() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = legacy_config(
        reflector_source(&env, &reflector, ReflectorBase::Usd),
        OracleSourceConfigOption::Some(redstone_source(&env, &adapter, "XLM")),
        OracleStrategy::PrimaryWithAnchor,
    );

    let lifted = lift_legacy(&env, &config);
    assert_eq!(lifted.sources.len(), 2);
    assert!(lifted.is_dual());

    // Order carries no meaning in the new model, but it must be deterministic.
    match lifted.sources.get_unchecked(0) {
        PriceSource::Feed(feed) => {
            assert_eq!(feed.provider.kind(), ProviderKind::Reflector);
            assert!(feed.provider.is_smoothed());
        }
        _ => panic!("expected a reflector feed first"),
    }
    match lifted.sources.get_unchecked(1) {
        PriceSource::Feed(feed) => assert_eq!(feed.provider.kind(), ProviderKind::RedStone),
        _ => panic!("expected a multi-feed second"),
    }
}

#[test]
fn test_anchor_is_dropped_when_strategy_is_single() {
    // A config carrying an anchor it never consults must not gain a second
    // opinion by being lifted; that would invent independence from nothing.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = legacy_config(
        reflector_source(&env, &reflector, ReflectorBase::Usd),
        OracleSourceConfigOption::Some(redstone_source(&env, &adapter, "XLM")),
        OracleStrategy::Single,
    );

    assert_eq!(lift_legacy(&env, &config).sources.len(), 1);
}

#[test]
fn test_quoted_base_lifts_to_a_scaled_source() {
    // The old model's one composition shape is a special case of the new one,
    // so this mapping is what proves the generalization is real.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let quote_token = Address::generate(&env);
    let config = legacy_config(
        reflector_source(&env, &reflector, ReflectorBase::Quoted(quote_token.clone())),
        OracleSourceConfigOption::None,
        OracleStrategy::Single,
    );

    match lift_legacy(&env, &config).sources.get_unchecked(0) {
        PriceSource::Scaled(scaled) => {
            assert_eq!(scaled.quote, PriceKey::Token(quote_token));
            // A legacy quoted source was never bounded; lifting carries the
            // absence forward rather than inventing a range.
            assert_eq!(scaled.min_factor_wad, 1);
            assert_eq!(scaled.max_factor_wad, i128::MAX);
        }
        _ => panic!("expected a scaled source"),
    }
}

#[test]
fn test_reflector_inherits_the_asset_bound_multi_feed_keeps_its_own() {
    // v1 resolved a Reflector leg's staleness to the asset default at read
    // time. Lifting writes that down instead of leaving it implicit, while a
    // multi-feed leg keeps the bound it always carried.
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = legacy_config(
        reflector_source(&env, &reflector, ReflectorBase::Usd),
        OracleSourceConfigOption::Some(redstone_source(&env, &adapter, "XLM")),
        OracleStrategy::PrimaryWithAnchor,
    );

    let lifted = lift_legacy(&env, &config);
    match lifted.sources.get_unchecked(0) {
        PriceSource::Feed(feed) => assert_eq!(feed.max_stale_seconds, ASSET_STALE),
        _ => panic!("expected a feed"),
    }
    match lifted.sources.get_unchecked(1) {
        PriceSource::Feed(feed) => assert_eq!(feed.max_stale_seconds, FEED_STALE),
        _ => panic!("expected a feed"),
    }
}

#[test]
fn test_lifted_multi_feed_takes_the_stricter_nature() {
    // Nature has no legacy counterpart, so lifting guesses - and must guess in
    // the direction that never makes a config look safer than it was verified
    // to be. Market is the strict choice: it carries the smoothing defect.
    let env = Env::default();
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    match lift_legacy(&env, &config).sources.get_unchecked(0) {
        PriceSource::Feed(feed) => {
            assert_eq!(feed.provider.nature(), FeedNature::Market);
            assert!(feed.provider.is_unsmoothed_market_leg());
        }
        _ => panic!("expected a feed"),
    }
}

#[test]
fn test_lifted_config_makes_no_independence_claim() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = legacy_config(
        reflector_source(&env, &reflector, ReflectorBase::Usd),
        OracleSourceConfigOption::Some(redstone_source(&env, &adapter, "XLM")),
        OracleStrategy::PrimaryWithAnchor,
    );

    assert_eq!(
        lift_legacy(&env, &config).independence,
        IndependencePolicy::RequireDisjoint
    );
}

// ---------------------------------------------------------------------------
// Dual-read: the new key wins, the legacy key stays reachable.
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_falls_back_to_the_legacy_entry() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    with_contract(&env, || {
        crate::storage::set_oracle_config(&env, &asset, &config);
        let key = PriceKey::Token(asset.clone());

        assert!(!is_migrated(&env, &key), "no new-shape entry written yet");
        let resolved = resolve_oracle(&env, &key).expect("legacy entry must stay readable");
        assert_eq!(resolved.sources.len(), 1);
        assert_eq!(resolved.asset_decimals, 7);
    });
}

#[test]
fn test_a_migrated_entry_shadows_the_legacy_one() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    with_contract(&env, || {
        crate::storage::set_oracle_config(&env, &asset, &config);
        let key = PriceKey::Token(asset.clone());

        // Migration is atomic from a reader's view: the new entry appearing is
        // the cutover, with no window where both or neither answer.
        let mut migrated = lift_legacy(&env, &config);
        migrated.asset_decimals = 18;
        set_oracle(&env, &key, &migrated);

        assert!(is_migrated(&env, &key));
        assert_eq!(resolve_oracle(&env, &key).unwrap().asset_decimals, 18);
    });
}

#[test]
fn test_reference_keys_have_no_legacy_form() {
    let env = Env::default();
    with_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "BTC"));
        // A reference price could not be expressed under the old model, so
        // absence is simply "not configured" - never a silent fallback.
        assert!(resolve_oracle(&env, &key).is_none());
        assert!(!is_migrated(&env, &key));
    });
}

#[test]
fn test_unmigrated_reports_only_keys_still_on_the_legacy_reader() {
    let env = Env::default();
    let migrated_asset = Address::generate(&env);
    let pending_asset = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    with_contract(&env, || {
        crate::storage::set_oracle_config(&env, &migrated_asset, &config);
        crate::storage::set_oracle_config(&env, &pending_asset, &config);

        let migrated_key = PriceKey::Token(migrated_asset.clone());
        let pending_key = PriceKey::Token(pending_asset.clone());
        set_oracle(&env, &migrated_key, &lift_legacy(&env, &config));

        let mut candidates = Vec::new(&env);
        candidates.push_back(migrated_key);
        candidates.push_back(pending_key.clone());

        let pending = unmigrated(&env, &candidates);
        assert_eq!(pending.len(), 1, "only the unwritten key is pending");
        assert_eq!(pending.get_unchecked(0), pending_key);
    });
}

#[test]
fn test_unmigrated_empties_once_every_candidate_is_written() {
    // The guard on retiring the legacy reader.
    let env = Env::default();
    let asset = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    with_contract(&env, || {
        let key = PriceKey::Token(asset.clone());
        set_oracle(&env, &key, &lift_legacy(&env, &config));

        let mut candidates = Vec::new(&env);
        candidates.push_back(key);
        assert_eq!(unmigrated(&env, &candidates).len(), 0);
    });
}

#[test]
fn test_writing_the_new_shape_leaves_the_legacy_entry_decodable() {
    // The whole point of the separate key variant: a `#[contracttype]` field-set
    // change traps on decode rather than returning None, and one undecodable
    // entry reverts an entire portfolio's health computation.
    let env = Env::default();
    let asset = Address::generate(&env);
    let adapter = Address::generate(&env);
    let config = single_redstone(&env, &adapter);

    with_contract(&env, || {
        crate::storage::set_oracle_config(&env, &asset, &config);
        let key = PriceKey::Token(asset.clone());
        set_oracle(&env, &key, &lift_legacy(&env, &config));

        let still_there = get_legacy(&env, &asset).expect("legacy entry must survive untouched");
        assert_eq!(still_there.strategy, OracleStrategy::Single);
        assert_eq!(still_there.asset_decimals, 7);
    });
}
