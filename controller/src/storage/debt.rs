use super::bump_shared;
use common::types::ControllerKey;
use soroban_sdk::{Address, Env};

pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

/// Persist the running isolated-debt counter for `asset`. Only writes
/// when the value carries information:
///   * `debt > 0` — write + bump TTL.
///   * `debt == 0` and an entry already exists — write 0 to record the
///     close (keeps a 0-valued marker so re-borrow stays a single write).
///   * `debt == 0` and no entry exists — no-op; reading via
///     [`get_isolated_debt`] still returns 0 via `unwrap_or`.
///
/// This keeps `IsolatedDebt(asset)` entries restricted to assets that
/// have actually carried isolated debt; non-isolated markets stay
/// off the persistent index.
pub fn set_isolated_debt(env: &Env, asset: &Address, debt: i128) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    let persistent = env.storage().persistent();
    if debt == 0 && !persistent.has(&key) {
        return;
    }
    persistent.set(&key, &debt);
    bump_shared(env, &key);
}
