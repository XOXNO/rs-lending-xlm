use super::*;
use soroban_sdk::testutils::Address as _;

// Dedup is first-seen-wins so batch results stay aligned with caller order.
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

// The same token listed on two hubs is priced once, at its first position.
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
