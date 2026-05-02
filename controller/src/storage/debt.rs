use super::bump_shared;
use common::types::ControllerKey;
use soroban_sdk::{Address, Env};

pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

pub fn set_isolated_debt(env: &Env, asset: &Address, debt: i128) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().set(&key, &debt);
    bump_shared(env, &key);
}
