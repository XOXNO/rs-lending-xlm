use super::renew_protocol_shared_key;
use common::types::ControllerKey;
use soroban_sdk::{Address, Env};

pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

// Persist the running isolated-debt counter for `asset`. User flows may update
// this shared accounting value, but they must not renew its TTL; keeper
// keepalive owns shared-state rent.
pub fn set_isolated_debt(env: &Env, asset: &Address, debt: i128) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    let persistent = env.storage().persistent();

    if debt <= 0 {
        persistent.remove(&key);
        return;
    }

    persistent.set(&key, &debt);
}

pub fn renew_isolated_debt_if_positive(env: &Env, asset: &Address) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    let persistent = env.storage().persistent();
    let Some(debt) = persistent.get::<_, i128>(&key) else {
        return;
    };

    if debt > 0 {
        renew_protocol_shared_key(env, &key);
    } else {
        persistent.remove(&key);
    }
}
