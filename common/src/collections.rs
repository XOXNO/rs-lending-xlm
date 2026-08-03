use soroban_sdk::{Address, Env, Vec};

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

#[cfg(test)]
#[path = "../tests/collections.rs"]
mod tests;
