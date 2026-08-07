use soroban_sdk::{Address, Env, IntoVal, Map, TryFromVal, Val, Vec};

use crate::types::HubAssetKey;

pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(&addr) {
        out.push_back(addr);
    }
}

pub fn unique_hub_tokens(env: &Env, hub_assets: &Vec<HubAssetKey>) -> Vec<Address> {
    let mut assets = Vec::new(env);
    for hub_asset in hub_assets.iter() {
        push_unique_address(&mut assets, hub_asset.asset);
    }
    assets
}

/// First-seen keys from `requested` that are absent from `cache`.
///
/// Preserves input order and drops duplicates already queued in the result.
///
/// The dedup is a linear scan of the result, so this is O(n^2) in
/// `requested.len()`. Callers must pass a bounded collection — current callers
/// are bounded by `PositionLimits::max_supply_positions` /
/// `max_borrow_positions`. Do not call this with an attacker-growable `Vec`:
/// on Soroban a quadratic scan is a budget-exhaustion revert, not a slowdown.
/// If an unbounded input is ever needed, dedup through a scratch `Map<K, ()>`.
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
