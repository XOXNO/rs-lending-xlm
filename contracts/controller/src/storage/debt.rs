//! Global isolated-debt ceiling counters.
//!
//! One `IsolatedDebt(asset)` persistent entry per isolated asset tracks the
//! aggregate USD debt (WAD) that has been borrowed against that asset while
//! any account is in isolation mode on it. The counter is mutated only
//! through the helpers here (and flushed via the cache in batch).

use common::types::ControllerKey;
use soroban_sdk::{Address, Env};

pub(crate) fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

pub(crate) fn set_isolated_debt(env: &Env, asset: &Address, debt: i128) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    let persistent = env.storage().persistent();

    if debt <= 0 {
        persistent.remove(&key);
        return;
    }

    persistent.set(&key, &debt);
}
