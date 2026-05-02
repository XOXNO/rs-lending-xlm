use super::bump_shared;
use common::errors::EModeError;
use common::types::{ControllerKey, EModeAssetConfig, EModeCategory};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

// ---------------------------------------------------------------------------
// EModeCategory (one entry per category)
// ---------------------------------------------------------------------------

pub fn get_emode_category(env: &Env, id: u32) -> EModeCategory {
    let key = ControllerKey::EModeCategory(id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound))
}

pub fn try_get_emode_category(env: &Env, id: u32) -> Option<EModeCategory> {
    let key = ControllerKey::EModeCategory(id);
    env.storage().persistent().get(&key)
}

pub fn set_emode_category(env: &Env, id: u32, cat: &EModeCategory) {
    let key = ControllerKey::EModeCategory(id);
    env.storage().persistent().set(&key, cat);
    bump_shared(env, &key);
}

// ---------------------------------------------------------------------------
// E-Mode asset memberships
//
// One ledger entry per category holding a `Map<Address, EModeAssetConfig>`.
// Per-asset reads/writes load + rewrite the whole side map; this keeps
// `remove_e_mode_category` to a single storage op instead of N orphan
// per-pair entries that Soroban can't enumerate.
// ---------------------------------------------------------------------------

fn emode_assets_key(category_id: u32) -> ControllerKey {
    ControllerKey::EModeAssets(category_id)
}

fn read_emode_assets_map(env: &Env, category_id: u32) -> Map<Address, EModeAssetConfig> {
    env.storage()
        .persistent()
        .get::<_, Map<Address, EModeAssetConfig>>(&emode_assets_key(category_id))
        .unwrap_or_else(|| Map::new(env))
}

fn write_emode_assets_map(env: &Env, category_id: u32, map: &Map<Address, EModeAssetConfig>) {
    let key = emode_assets_key(category_id);
    let persistent = env.storage().persistent();
    if map.is_empty() {
        persistent.remove(&key);
        return;
    }
    persistent.set(&key, map);
    bump_shared(env, &key);
}

pub fn get_emode_asset(env: &Env, category_id: u32, asset: &Address) -> Option<EModeAssetConfig> {
    read_emode_assets_map(env, category_id).get(asset.clone())
}

pub fn set_emode_asset(env: &Env, category_id: u32, asset: &Address, config: &EModeAssetConfig) {
    let mut map = read_emode_assets_map(env, category_id);
    map.set(asset.clone(), config.clone());
    write_emode_assets_map(env, category_id, &map);
}

pub fn remove_emode_asset(env: &Env, category_id: u32, asset: &Address) {
    let mut map = read_emode_assets_map(env, category_id);
    map.remove(asset.clone());
    write_emode_assets_map(env, category_id, &map);
}

/// Returns the full side map for a category. Used by category-deprecation
/// flows that need to enumerate every member asset to clean up reverse
/// indices.
pub fn get_emode_assets(env: &Env, category_id: u32) -> Map<Address, EModeAssetConfig> {
    read_emode_assets_map(env, category_id)
}

/// Drops the entire side map for a category in a single storage operation.
pub fn remove_emode_assets(env: &Env, category_id: u32) {
    env.storage()
        .persistent()
        .remove(&emode_assets_key(category_id));
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
