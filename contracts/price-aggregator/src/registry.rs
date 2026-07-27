//! Price registry: keyed persistent storage for [`AssetOracle`].
//!
//! One key space, one shape. A [`PriceKey`] covers both real assets
//! ([`PriceKey::Token`]) and pure reference prices ([`PriceKey::Ref`]), so a
//! reference with no token on any ledger is storable beside a listed market
//! without a second table or a second lookup path.
//!
//! Entries live in the protocol-shared persistent tier with their TTL renewed
//! on every read and write.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::types::{AssetOracle, PriceKey};
use soroban_sdk::{contracttype, Env};

#[contracttype]
enum AggregatorKey {
    /// [`AssetOracle`] under a [`PriceKey`]. The variant wraps the key rather
    /// than an `Address` so a reference price, which has no token, is storable
    /// beside a listed market without a second table.
    Oracle(PriceKey),
}

// ---------------------------------------------------------------------------
// Current shape
// ---------------------------------------------------------------------------

/// Stored [`AssetOracle`] for `key`, renewing its shared-tier TTL on hit.
pub(crate) fn get_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let oracle: Option<AssetOracle> = env.storage().persistent().get(&storage_key);
    if oracle.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    oracle
}

pub(crate) fn set_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    env.storage().persistent().set(&storage_key, oracle);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Test-only: removes a key's oracle, disabling pricing for it.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn remove_oracle(env: &Env, key: &PriceKey) {
    env.storage()
        .persistent()
        .remove(&AggregatorKey::Oracle(key.clone()));
}

/// The oracle to price `key` with, if one is configured.
///
/// A thin alias over [`get_oracle`] so every read path has one name to call and
/// a future lookup rule lands in one place.
pub(crate) fn resolve_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    get_oracle(env, key)
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
