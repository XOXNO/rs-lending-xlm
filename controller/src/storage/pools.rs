use super::bump_shared;
use common::types::ControllerKey;
use soroban_sdk::{Address, Env, Vec};

/// Returns the asset addresses of every market the controller manages.
/// Pool addresses are resolved via `MarketConfig.pool_address` so the
/// list value stays a flat `Vec<Address>`.
pub fn get_pools_list(env: &Env) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsList)
        .unwrap_or_else(|| Vec::new(env))
}

/// Number of pools — `vec.len()` is authoritative; no separate
/// `PoolsCount` entry is maintained. Exposed for test fixtures and any
/// future enumeration entrypoint.
/// Bumps the single `PoolsList` entry. No-ops when no pools exist yet.
pub fn bump_pools_list(env: &Env) {
    let key = ControllerKey::PoolsList;
    if env.storage().persistent().has(&key) {
        bump_shared(env, &key);
    }
}

/// Appends `asset` to the asset list. The pool address is implicit via
/// `MarketConfig(asset).pool_address` and is not stored here.
///
/// `_pool` is accepted for backwards-compatible callers but ignored —
/// kept on the signature so the public API doesn't shift the call sites
/// in `router.rs::create_liquidity_pool`.
pub fn add_to_pools_list(env: &Env, asset: &Address, _pool: &Address) {
    let mut list = get_pools_list(env);
    list.push_back(asset.clone());
    let key = ControllerKey::PoolsList;
    env.storage().persistent().set(&key, &list);
    bump_shared(env, &key);
}
