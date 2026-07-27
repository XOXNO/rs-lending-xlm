use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};

fn token(env: &Env) -> PriceKey {
    PriceKey::Token(Address::generate(env))
}

#[test]
fn test_price_memo_round_trips_per_key() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let key = token(&env);

    assert!(cache.cached_key_price(&key).is_none());
    cache.store_key_price(
        &key,
        PriceFeedRaw {
            price_wad: 7,
            asset_decimals: 8,
            timestamp: 100,
        },
    );
    assert_eq!(cache.cached_key_price(&key).unwrap().price_wad, 7);
}

#[test]
fn test_price_and_status_memos_are_separate() {
    // A soft status may describe a price the hard path would reject, so sharing
    // one map would be a route for an unusable reading to reach a fail-closed
    // caller.
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let key = token(&env);

    cache.store_key_status(&key, PriceStatus::unusable());
    assert!(cache.cached_key_status(&key).is_some());
    assert!(
        cache.cached_key_price(&key).is_none(),
        "a stored status must never satisfy a price lookup"
    );
}

#[test]
fn test_memos_do_not_alias_across_key_variants() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let reference = PriceKey::Ref(Symbol::new(&env, "BTC"));
    let key = token(&env);

    cache.store_key_price(
        &key,
        PriceFeedRaw {
            price_wad: 1,
            asset_decimals: 8,
            timestamp: 0,
        },
    );
    assert!(cache.cached_key_price(&reference).is_none());
}

#[test]
fn test_resolution_stack_tracks_entry_and_exit() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let key = token(&env);

    assert!(!cache.is_price_key_resolving(&key));
    cache.push_price_key(&key);
    assert!(cache.is_price_key_resolving(&key));
    cache.pop_price_key();
    assert!(!cache.is_price_key_resolving(&key));
}

#[test]
#[should_panic]
fn test_re_entering_a_key_reverts_as_a_cycle() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let key = token(&env);
    cache.push_price_key(&key);
    cache.push_price_key(&key);
}

#[test]
fn test_nested_distinct_keys_are_allowed() {
    // Composition is legal; only re-entering the same key is a cycle.
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let outer = token(&env);
    let inner = PriceKey::Ref(Symbol::new(&env, "BTC"));

    cache.push_price_key(&outer);
    cache.push_price_key(&inner);
    cache.pop_price_key();
    cache.pop_price_key();
    assert!(!cache.is_price_key_resolving(&outer));
}

#[test]
fn test_ledger_timestamp_is_sampled_once() {
    // Every freshness judgement in one transaction must be made against the
    // same instant, or two sources read microseconds apart could disagree about
    // what "now" is.
    let env = Env::default();
    let cache = ResolutionContext::new(&env);
    let sampled = cache.ledger_timestamp_secs();
    assert_eq!(cache.ledger_timestamp_secs(), sampled);
}
