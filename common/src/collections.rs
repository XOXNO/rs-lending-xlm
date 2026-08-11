//! Generic collection helpers for building deduplicated `Vec`s and diffing
//! requested keys against a cache `Map`.

use soroban_sdk::{Address, Env, IntoVal, Map, TryFromVal, Val, Vec};

use crate::types::HubAssetKey;

/// Appends `addr` to `out` unless it is already present.
pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(&addr) {
        out.push_back(addr);
    }
}

/// Returns the distinct asset addresses across `hub_assets`, in first-seen order.
pub fn unique_hub_tokens(env: &Env, hub_assets: &Vec<HubAssetKey>) -> Vec<Address> {
    let mut assets = Vec::new(env);
    for hub_asset in hub_assets.iter() {
        push_unique_address(&mut assets, hub_asset.asset);
    }
    assets
}

/// Returns the entries of `requested` that are not keys of `cache`, in
/// first-seen order, with duplicates removed.
///
/// Dedup is a linear scan of the result (O(n²) in `requested.len()`). On
/// Soroban that is a budget-exhaustion revert, not a slowdown. Callers must
/// pass a bounded collection (current callers are capped by
/// `PositionLimits::max_supply_positions` / `max_borrow_positions`). Do not
/// call with an attacker-growable `Vec`; use a scratch `Map<K, ()>` if an
/// unbounded input is ever required.
pub fn collect_uncached_keys<K, V>(env: &Env, requested: &Vec<K>, cache: &Map<K, V>) -> Vec<K>
where
    K: Clone + IntoVal<Env, Val> + TryFromVal<Env, Val>,
    V: IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    let mut missing = Vec::new(env);
    for key in requested.iter() {
        if !cache.contains_key(key.clone()) && !missing.contains(&key) {
            missing.push_back(key);
        }
    }
    missing
}

#[cfg(test)]
#[path = "../tests/collections.rs"]
mod tests;
