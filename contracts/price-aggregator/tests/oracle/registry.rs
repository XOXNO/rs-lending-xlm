use super::*;
use common::types::{
    FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleAssetRef, OracleReadMode,
    OracleTolerance, PriceSource, ProviderKind, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String, Symbol, Vec};

use crate::test_support::{NonUsdReflector, TwapReflector};
use crate::PriceAggregator;

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
    // The reason the key space is not just an Address: BTC has no token on any
    // ledger, and must still be storable and priceable.
    let env = Env::default();
    with_contract(&env, || {
        let key = PriceKey::Ref(Symbol::new(&env, "BTC"));
        store_oracle(&env, &key, &oracle(&env, 0));
        assert!(get_oracle(&env, &key).is_some());
    });
}

#[test]
fn test_token_and_reference_keys_do_not_alias() {
    // Distinct variants must produce distinct storage keys, or a reference
    // price could silently shadow a listed market's config.
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

// ---------------------------------------------------------------------------
// Attestation
// ---------------------------------------------------------------------------
//
// Validation checks that a config is well-formed; attestation checks that it
// matches the provider it names. A config can be perfectly well-formed and still
// describe a contract that reports different decimals, or quotes a different
// currency — neither of which any later read would notice.

/// A single Reflector-backed TWAP config over `contract`, declaring `decimals`.
/// Two records, one resolution apart, comfortably inside the ceiling.
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
        // The fixture's two samples mean to 2 WAD, and a single source has to
        // sit inside MAX_SINGLE_SOURCE_SANITY_BAND_BPS of its own midpoint.
        min_sanity_price_wad: TWAP_MEAN_WAD - TWAP_MEAN_WAD / 20,
        max_sanity_price_wad: TWAP_MEAN_WAD + TWAP_MEAN_WAD / 20,
    }
}

/// What [`TwapReflector`]'s two samples average to, in WAD.
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
    // The control for every rejection below: same shape, nothing disagreeing.
    // It is also what makes the base check falsifiable — a guard that always
    // rejected would fail here rather than passing every negative case.
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
    // The stub reports 14. A config claiming 8 would rescale every price by
    // 1e6 — a config-time typo that no read path can detect.
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
    // A scaled source reaches its provider through `factor`, so attestation has
    // to descend into it. Skipping that arm would leave every wrapper-asset
    // config unattested while direct feeds stayed covered.
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
