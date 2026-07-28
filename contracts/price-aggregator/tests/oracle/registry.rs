use super::*;
use common::types::{
    FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance, PriceSource,
    ProviderKind, ProviderRef,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Symbol, Vec};

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
