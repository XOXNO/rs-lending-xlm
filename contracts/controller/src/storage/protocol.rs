use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits, PositionManagerConfig};

/// Extends the controller contract's instance storage TTL using the
/// protocol-wide instance threshold and bump constants.
pub(crate) use common::ttl::renew_instance as renew_controller_instance;

use soroban_sdk::{panic_with_error, Address, Env, IntoVal, TryFromVal, Val};

use crate::constants::{
    self, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};

/// Returns shared persistent Blend approval, defaulting to false.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    get_shared(env, &ControllerKey::BlendPoolAllowed(pool.clone())).unwrap_or(false)
}

/// Stores Blend approval with shared TTL renewal, or removes it on revocation.
pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = ControllerKey::BlendPoolAllowed(pool.clone());
    if approved {
        set_shared(env, &key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

/// Returns the instance pool address or fails with `PoolNotInitialized`.
pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Returns the instance pool address, or `None` when unset.
pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

/// Stores the pool address in instance storage.
pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

/// Returns the instance swap aggregator or fails with `AggregatorNotSet`.
pub(crate) fn get_swap_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::SwapAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Stores the swap aggregator in instance storage.
pub(crate) fn set_swap_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::SwapAggregator, addr);
}

/// Returns the instance price aggregator or fails with `AggregatorNotSet`.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Stores the price aggregator in instance storage.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PriceAggregator, addr);
}

/// Returns the instance accumulator address, or `None` when unset.
pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

/// Stores the accumulator address in instance storage.
pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

/// Returns instance position limits or fails with `PositionLimitsNotSet`.
pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

/// Stores supply and debt position limits in instance storage.
pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

/// Returns the instance LTV-weighted collateral floor in USD WAD,
/// defaulting to `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`.
pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

/// Stores the LTV-weighted borrow collateral floor in USD WAD.
pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

/// Returns the instance NFT address or fails with `PositionNftNotSet`.
pub(crate) fn get_position_nft(env: &Env) -> Address {
    try_get_position_nft(env)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionNftNotSet))
}

/// Returns the instance NFT address, or `None` when unset.
pub(crate) fn try_get_position_nft(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::PositionNft)
}

/// Stores the NFT address in instance storage.
pub(crate) fn set_position_nft(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionNft, addr);
}

/// Returns shared persistent manager config, renewing TTL when present.
pub(crate) fn get_position_manager(env: &Env, addr: &Address) -> Option<PositionManagerConfig> {
    get_shared(env, &ControllerKey::PositionManager(addr.clone()))
}

/// Stores active manager config with shared TTL renewal; removes inactive config.
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

/// Extends persistent TTL with the supplied threshold and bump ledger counts.
fn renew_persistent_key(env: &Env, key: &ControllerKey, threshold: u32, bump: u32) {
    env.storage().persistent().extend_ttl(key, threshold, bump);
}

/// Extends persistent TTL with the per-account user window.
pub(super) fn renew_user_key(env: &Env, key: &ControllerKey) {
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

/// Increments an instance counter from zero when absent; fails on overflow.
pub(super) fn increment_counter(env: &Env, key: &ControllerKey) -> u32 {
    let current: u32 = env.storage().instance().get(key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(key, &next);
    next
}

/// Reads a persistent value and renews shared TTL only when present.
pub(super) fn get_shared<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    value
}

/// Writes a persistent value and renews shared TTL.
pub(super) fn set_shared<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_persistent_key(env, key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Reads a persistent value and renews user TTL only when present.
pub(super) fn get_user<V: TryFromVal<Env, Val>>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
    }
    value
}

/// Writes a persistent value and renews user TTL.
pub(super) fn set_user<V: IntoVal<Env, Val>>(env: &Env, key: &ControllerKey, value: &V) {
    env.storage().persistent().set(key, value);
    renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}
