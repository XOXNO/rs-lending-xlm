use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits, PositionManagerConfig};

/// Extends the controller contract's instance storage TTL using the
/// protocol-wide instance threshold and bump constants.
pub(crate) use common::ttl::renew_instance as renew_controller_instance;

use soroban_sdk::{panic_with_error, Address, Env, IntoVal, TryFromVal, Val};

use crate::constants::{
    self, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};

/// Reads whether `pool` is an approved Blend pool from shared persistent storage, defaulting to `false` if unset.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    get_shared(env, &ControllerKey::BlendPoolAllowed(pool.clone())).unwrap_or(false)
}

/// Sets or clears a Blend pool's approval flag in shared persistent storage; clearing the flag removes the entry rather than storing `false`.
pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = ControllerKey::BlendPoolAllowed(pool.clone());
    if approved {
        set_shared(env, &key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

/// Reads the controller's configured pool address from instance storage, panicking with `PoolNotInitialized` if unset.
pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Reads the controller's configured pool address from instance storage, or `None` if unset.
pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

/// Writes the controller's pool address to instance storage.
pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

/// Reads the configured swap aggregator address from instance storage, panicking with `AggregatorNotSet` if unset.
pub(crate) fn get_swap_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::SwapAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Writes the swap aggregator address to instance storage.
pub(crate) fn set_swap_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::SwapAggregator, addr);
}

/// Reads the configured price aggregator address from instance storage, panicking with `AggregatorNotSet` if unset.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Writes the price aggregator address to instance storage.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PriceAggregator, addr);
}

/// Reads the configured accumulator address from instance storage, or `None` if unset.
pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

/// Writes the accumulator address to instance storage.
pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

/// Reads the configured maximum supply/borrow position counts from instance storage, panicking with `PositionLimitsNotSet` if unset.
pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

/// Writes the maximum supply/borrow position counts to instance storage.
pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

/// Reads the minimum collateral value (WAD) required to open a borrow from instance storage, defaulting to `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD` if unset.
pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

/// Writes the minimum borrow-collateral value floor (WAD) to instance storage.
pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

/// Reads the position-NFT contract address from instance storage, panicking
/// with `PositionNftNotSet` if the NFT has not been deployed.
pub(crate) fn get_position_nft(env: &Env) -> Address {
    try_get_position_nft(env)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionNftNotSet))
}

/// Reads the position-NFT contract address from instance storage, or `None` if unset.
pub(crate) fn try_get_position_nft(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::PositionNft)
}

/// Writes the position-NFT contract address to instance storage.
pub(crate) fn set_position_nft(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionNft, addr);
}

/// Reads a position manager's configuration from shared persistent storage keyed by its address, or `None` if not registered.
pub(crate) fn get_position_manager(env: &Env, addr: &Address) -> Option<PositionManagerConfig> {
    get_shared(env, &ControllerKey::PositionManager(addr.clone()))
}

/// Writes a position manager's configuration to shared persistent storage if `is_active` is set, or removes the entry otherwise.
pub(crate) fn set_position_manager(env: &Env, addr: &Address, config: &PositionManagerConfig) {
    let key = ControllerKey::PositionManager(addr.clone());
    if config.is_active {
        set_shared(env, &key, config);
    } else {
        env.storage().persistent().remove(&key);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/protocol.rs"]
mod tests;

/// Extends a persistent storage key's TTL using the given threshold and bump ledger counts.
fn renew_persistent_key(env: &Env, key: &ControllerKey, threshold: u32, bump: u32) {
    env.storage().persistent().extend_ttl(key, threshold, bump);
}

/// Extends a persistent key's TTL using the per-account (user) threshold and bump constants.
pub(super) fn renew_user_key(env: &Env, key: &ControllerKey) {
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Reads and returns the counter at `key`, incremented by one and stored back.
/// Panics with `MathOverflow` on overflow.
pub(super) fn increment_counter(env: &Env, key: &ControllerKey) -> u32 {
    let current: u32 = env.storage().instance().get(key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(key, &next);
    next
}

// Raw key-value access stays inside the storage module: every crate-visible
// accessor is typed, so a wrongly-typed write under a typed key cannot compile.
/// Reads a persistent value under `key` using the shared (protocol/hub/spoke-wide) TTL thresholds, extending its TTL only when a value is present.
pub(super) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    value
}

/// Writes a persistent value under `key` using the shared (protocol/hub/spoke-wide) TTL thresholds.
pub(super) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_persistent_key(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Reads a persistent value under `key` using the per-account (user) TTL thresholds, extending its TTL only when a value is present.
pub(super) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
    }
    value
}

/// Writes a persistent value under `key` using the per-account (user) TTL thresholds.
pub(super) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}
