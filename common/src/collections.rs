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
/// Dedup scans the result, so cost is O(n²) in `requested.len()` and an
/// oversized input reverts on budget exhaustion. Callers must bound
/// `requested`: account positions are capped by `PositionLimits`, and view
/// inputs by the controller's `MAX_VIEW_INPUTS`.
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
