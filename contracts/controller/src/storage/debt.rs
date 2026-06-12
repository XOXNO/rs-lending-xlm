//! Global isolated-debt ceiling counters.
//!
//! One `IsolatedDebt(asset)` persistent entry per isolated asset tracks the
//! aggregate USD debt (WAD) that has been borrowed against that asset while
//! any account is in isolation mode on it. The counter is mutated only
//! through the helpers here (and flushed via the cache in batch).

use super::{renew_protocol_shared_key, renew_user_key};
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
    // Shared-tier TTL bump on every write, matching `set_market_config` and the
    // other protocol-shared keys. Skipped on the remove branch above because
    // `extend_ttl` panics on a missing key (soroban-sdk 26.x).
    renew_protocol_shared_key(env, &key);
}

/// USD WAD principal that debt position `(account_id, asset)` contributed to
/// the isolated-debt ceiling at borrow time. `0` when absent.
pub(crate) fn get_isolated_basis(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedBasis(account_id, asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

/// Upserts the per-position isolation basis, removing the entry when it reaches
/// zero so a fully repaid position leaves no dust key behind.
pub(crate) fn set_isolated_basis(env: &Env, account_id: u64, asset: &Address, basis: i128) {
    let key = ControllerKey::IsolatedBasis(account_id, asset.clone());
    let persistent = env.storage().persistent();

    if basis <= 0 {
        persistent.remove(&key);
        return;
    }

    persistent.set(&key, &basis);
    // User-tier TTL bump, matching the account's `BorrowPositions` lifetime.
    renew_user_key(env, &key);
}
