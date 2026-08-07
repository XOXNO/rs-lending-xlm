use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Map, Vec};

#[test]
fn push_unique_dedups_preserving_order() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let mut out: Vec<Address> = Vec::new(&env);
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, b.clone());
    assert_eq!(out.len(), 2);
    assert_eq!(out.get_unchecked(0), a);
    assert_eq!(out.get_unchecked(1), b);
}

#[test]
fn unique_hub_tokens_collapses_same_token_across_hubs() {
    let env = Env::default();
    let shared = Address::generate(&env);
    let other = Address::generate(&env);
    let keys = Vec::from_array(
        &env,
        [
            HubAssetKey {
                hub_id: 0,
                asset: shared.clone(),
            },
            HubAssetKey {
                hub_id: 1,
                asset: other.clone(),
            },
            HubAssetKey {
                hub_id: 2,
                asset: shared.clone(),
            },
        ],
    );

    let tokens = unique_hub_tokens(&env, &keys);
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens.get_unchecked(0), shared);
    assert_eq!(tokens.get_unchecked(1), other);
}

#[test]
fn collect_uncached_keys_empty_request() {
    let env = Env::default();
    let cache: Map<Address, u32> = Map::new(&env);
    let requested: Vec<Address> = Vec::new(&env);
    let missing = collect_uncached_keys(&env, &requested, &cache);
    assert_eq!(missing.len(), 0);
}

#[test]
fn collect_uncached_keys_all_cached_is_empty() {
    let env = Env::default();
    let a = Address::generate(&env);
    let mut cache = Map::new(&env);
    cache.set(a.clone(), 1u32);
    let requested = Vec::from_array(&env, [a.clone(), a.clone()]);
    let missing = collect_uncached_keys(&env, &requested, &cache);
    assert_eq!(missing.len(), 0);
}

#[test]
fn collect_uncached_keys_dedups_preserving_first_seen_order() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    let mut cache = Map::new(&env);
    cache.set(b.clone(), 1u32);
    let requested = Vec::from_array(
        &env,
        [a.clone(), b.clone(), a.clone(), c.clone(), c.clone()],
    );
    let missing = collect_uncached_keys(&env, &requested, &cache);
    assert_eq!(missing.len(), 2);
    assert_eq!(missing.get_unchecked(0), a);
    assert_eq!(missing.get_unchecked(1), c);
}
