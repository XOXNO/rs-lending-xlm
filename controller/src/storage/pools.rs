use super::bump_shared;
use common::errors::GenericError;
use common::types::ControllerKey;
use soroban_sdk::{panic_with_error, Address, Env};

pub fn get_pools_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsCount)
        .unwrap_or(0u32)
}

pub fn set_pools_count(env: &Env, count: u32) {
    let key = ControllerKey::PoolsCount;
    env.storage().persistent().set(&key, &count);
    bump_shared(env, &key);
}

#[cfg(test)]
pub fn get_pools_list_entry(env: &Env, idx: u32) -> (Address, Address) {
    let key = ControllerKey::PoolsList(idx);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolsListNotFound))
}

pub fn bump_pools_list(env: &Env) {
    let count = get_pools_count(env);
    bump_shared(env, &ControllerKey::PoolsCount);
    for i in 0..count {
        bump_shared(env, &ControllerKey::PoolsList(i));
    }
}

pub fn add_to_pools_list(env: &Env, asset: &Address, pool: &Address) {
    let count = get_pools_count(env);
    let key = ControllerKey::PoolsList(count);
    env.storage()
        .persistent()
        .set(&key, &(asset.clone(), pool.clone()));
    bump_shared(env, &key);
    set_pools_count(
        env,
        count
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    );
}
