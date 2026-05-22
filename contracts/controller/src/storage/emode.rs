use super::renew_protocol_shared_key;
use common::errors::EModeError;
use common::types::{ControllerKey, EModeAssetConfig, EModeCategoryRaw};
use soroban_sdk::{panic_with_error, Address, Env};

pub(crate) fn get_emode_category(env: &Env, id: u32) -> EModeCategoryRaw {
    env.storage()
        .persistent()
        .get(&ControllerKey::EModeCategory(id))
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
    cat.assets.set(asset.clone(), config.clone());
    set_emode_category(env, category_id, &cat);
}

pub(crate) fn remove_emode_asset(env: &Env, category_id: u32, asset: &Address) {
    if let Some(mut cat) = try_get_emode_category(env, category_id) {
        cat.assets.remove(asset.clone());
        set_emode_category(env, category_id, &cat);
    }
}
