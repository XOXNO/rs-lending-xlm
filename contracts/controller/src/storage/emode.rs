//! E-mode category and membership storage.
//!
//! Categories are versioned records that can be deprecated. Membership
//! (`EModeAssetConfig`) is stored per (category, asset) under shared keys
//! so that a deprecated category can be left behind without touching any
//! account that ever used it (core of ADR 0008).

use super::renew_protocol_shared_key;
use common::constants::MAX_EMODE_ASSETS_PER_CATEGORY;
use common::errors::EModeError;
use common::types::{ControllerKey, EModeAssetConfig, EModeCategoryRaw};
use soroban_sdk::{panic_with_error, Address, Env};

pub(crate) fn get_emode_category(env: &Env, id: u32) -> EModeCategoryRaw {
    try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound))
}

pub(crate) fn try_get_emode_category(env: &Env, id: u32) -> Option<EModeCategoryRaw> {
    env.storage()
        .persistent()
        .get(&ControllerKey::EModeCategory(id))
}

pub(crate) fn set_emode_category(env: &Env, id: u32, cat: &EModeCategoryRaw) {
    let key = ControllerKey::EModeCategory(id);
    env.storage().persistent().set(&key, cat);
    renew_protocol_shared_key(env, &key);
}

pub(crate) fn get_emode_asset(
    env: &Env,
    category_id: u32,
    asset: &Address,
) -> Option<EModeAssetConfig> {
    try_get_emode_category(env, category_id).and_then(|cat| cat.assets.get(asset.clone()))
}

pub(crate) fn set_emode_asset(
    env: &Env,
    category_id: u32,
    asset: &Address,
    config: &EModeAssetConfig,
) {
    let mut cat = try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    // Cap applies to inserts only — updates leave cardinality unchanged.
    let is_new = !cat.assets.contains_key(asset.clone());
    if is_new && cat.assets.len() >= MAX_EMODE_ASSETS_PER_CATEGORY {
        panic_with_error!(env, EModeError::EModeAssetsLimitReached);
    }
    cat.assets.set(asset.clone(), config.clone());
    set_emode_category(env, category_id, &cat);
}

pub(crate) fn remove_emode_asset(env: &Env, category_id: u32, asset: &Address) {
    if let Some(mut cat) = try_get_emode_category(env, category_id) {
        cat.assets.remove(asset.clone());
        set_emode_category(env, category_id, &cat);
    }
}
