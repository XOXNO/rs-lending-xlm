use super::bump_shared;
use common::errors::EModeError;
use common::types::{ControllerKey, EModeAssetConfig, EModeCategory};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

// ---------------------------------------------------------------------------
// EModeCategory (one entry per category — params + member-asset map)
// ---------------------------------------------------------------------------

fn category_key(id: u32) -> ControllerKey {
    ControllerKey::EModeCategory(id)
}

pub fn get_emode_category(env: &Env, id: u32) -> EModeCategory {
    env.storage()
        .persistent()
        .get(&category_key(id))
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound))
}

pub fn try_get_emode_category(env: &Env, id: u32) -> Option<EModeCategory> {
    env.storage().persistent().get(&category_key(id))
}

pub fn set_emode_category(env: &Env, id: u32, cat: &EModeCategory) {
    let key = category_key(id);
    env.storage().persistent().set(&key, cat);
    bump_shared(env, &key);
}

// ---------------------------------------------------------------------------
// E-Mode asset memberships — read/written through the embedded `assets` map
// on the [`EModeCategory`] entry. Single ledger entry per category; per-asset
// ops load + rewrite the whole entry.
// ---------------------------------------------------------------------------

pub fn get_emode_asset(env: &Env, category_id: u32, asset: &Address) -> Option<EModeAssetConfig> {
    try_get_emode_category(env, category_id).and_then(|cat| cat.assets.get(asset.clone()))
}

pub fn set_emode_asset(env: &Env, category_id: u32, asset: &Address, config: &EModeAssetConfig) {
    let mut cat = try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    cat.assets.set(asset.clone(), config.clone());
    set_emode_category(env, category_id, &cat);
}

pub fn remove_emode_asset(env: &Env, category_id: u32, asset: &Address) {
    if let Some(mut cat) = try_get_emode_category(env, category_id) {
        cat.assets.remove(asset.clone());
        set_emode_category(env, category_id, &cat);
    }
}

// ---------------------------------------------------------------------------
// Reverse index: per-asset list of categories
// ---------------------------------------------------------------------------

pub fn get_asset_emodes(env: &Env, asset: &Address) -> Vec<u32> {
    let key = ControllerKey::AssetEModes(asset.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_asset_emodes(env: &Env, asset: &Address, categories: &Vec<u32>) {
    let key = ControllerKey::AssetEModes(asset.clone());
    env.storage().persistent().set(&key, categories);
    bump_shared(env, &key);
}
