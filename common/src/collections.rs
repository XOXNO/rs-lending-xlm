//! Order-preserving dedup helpers for Soroban vectors.
//!
//! Deduping is first-seen-wins so batch results stay aligned with caller input
//! order. Concrete on `Address` rather than generic: the contracts are built at
//! `opt-level = "z"` and a monomorphized copy per element type costs wasm bytes.

use soroban_sdk::{Address, Env, Vec};

use crate::types::HubAssetKey;

/// Appends `addr` to `out` if absent, preserving first-seen order.
pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(&addr) {
        out.push_back(addr);
    }
}

/// Token addresses behind a hub-asset batch, deduped in first-seen order.
///
/// The same token can appear on several hubs, and on both sides of one
/// account's book; pricing it once per batch is what callers want.
pub fn unique_hub_tokens(env: &Env, hub_assets: &Vec<HubAssetKey>) -> Vec<Address> {
    let mut assets = Vec::new(env);
    for hub_asset in hub_assets.iter() {
        push_unique_address(&mut assets, hub_asset.asset);
    }
    assets
}

#[cfg(test)]
#[path = "../tests/collections.rs"]
mod tests;
