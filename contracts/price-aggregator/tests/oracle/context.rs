use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Symbol};

fn token(env: &Env) -> PriceKey {
    PriceKey::Token(Address::generate(env))
}

#[test]
fn test_price_memo_round_trips_per_key() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);

    assert!(session.cached_price(&key).is_none());
    session.store_price(
        &key,
        PriceFeedRaw {
            price_wad: 7,
            asset_decimals: 8,
            timestamp: 100,
        },
    );
    assert_eq!(session.cached_price(&key).unwrap().price_wad, 7);
}

#[test]
fn test_price_and_status_memos_are_separate() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);

    session.store_status(&key, PriceStatus::unusable());
    assert!(session.cached_status(&key).is_some());
    assert!(
        session.cached_price(&key).is_none(),
        "a stored status must never satisfy a price lookup"
    );
}

#[test]
fn test_cycle_stack_rejects_reentry() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let key = token(&env);
    session.push_key(&key);
    // Second push panics — covered by engine cycle tests; here only is_resolving.
    assert!(session.is_resolving(&key));
    session.pop_key();
    assert!(!session.is_resolving(&key));
}

#[test]
fn test_distinct_keys_do_not_collide() {
    let env = Env::default();
    let mut session = Session::new(&env);
    let a = token(&env);
    let b = PriceKey::Ref(Symbol::new(&env, "BTC"));
    session.push_key(&a);
    assert!(!session.is_resolving(&b));
    session.pop_key();
}
