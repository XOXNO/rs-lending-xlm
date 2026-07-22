use super::ResolutionContext;
use common::types::PriceFeedRaw;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn price_cache_stores_and_reports_entries() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let asset = Address::generate(&env);
    assert!(!cache.has_price(&asset));
    assert!(cache.cached_price(&asset).is_none());

    cache.store_price(
        &asset,
        PriceFeedRaw {
            price_wad: 42,
            asset_decimals: 7,
            timestamp: 1,
        },
    );

    assert!(cache.has_price(&asset));
    let cached = cache.cached_price(&asset).expect("stored feed");
    assert_eq!(cached.price_wad, 42);
    assert_eq!(cached.asset_decimals, 7);
    assert_eq!(cached.timestamp, 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #225)")]
fn push_resolution_traps_reentry() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let asset = Address::generate(&env);
    cache.push_resolution(&asset);
    cache.push_resolution(&asset);
}

#[test]
fn pop_resolution_releases_the_cycle_guard() {
    let env = Env::default();
    let mut cache = ResolutionContext::new(&env);
    let asset = Address::generate(&env);
    cache.push_resolution(&asset);
    cache.pop_resolution();
    // Released: re-entering the same asset must not trip the cycle guard.
    cache.push_resolution(&asset);
}
