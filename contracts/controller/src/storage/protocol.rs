//! Storage accessors for protocol-wide controller configuration: the Blend pool allowlist,
//! integration addresses (pool, swap aggregator, price aggregator, accumulator), the
//! account nonce counter, position limits, and per-position-manager configuration.

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits, PositionManagerConfig};

use soroban_sdk::{panic_with_error, Address, Env};

use crate::constants;
use crate::storage::{get_shared, set_shared};

/// Returns whether `pool` is on the approved Blend pool allowlist. Defaults to `false` if unset.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    get_shared(env, &ControllerKey::BlendPoolAllowed(pool.clone())).unwrap_or(false)
}

/// Sets whether `pool` is approved. Setting `approved` to `false` removes the storage key
/// instead of storing a negative flag.
pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = ControllerKey::BlendPoolAllowed(pool.clone());
    if approved {
        set_shared(env, &key, &true);
    } else {
        env.storage().persistent().remove(&key);
    }
}

/// Reads the configured liquidity-pool address. Panics with `PoolNotInitialized` if unset.
pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Reads the configured liquidity-pool address. Returns `None` if unset.
pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

/// Writes the liquidity-pool address.
pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

/// Reads the configured swap aggregator address. Panics with `AggregatorNotSet` if unset.
pub(crate) fn get_swap_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::SwapAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Writes the swap aggregator address.
pub(crate) fn set_swap_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::SwapAggregator, addr);
}

/// Reads the configured price aggregator address. Panics with `AggregatorNotSet` if unset.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Writes the price aggregator address.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PriceAggregator, addr);
}

/// Reads the configured accumulator address. Returns `None` if unset.
pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

/// Writes the accumulator address.
pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

/// Reads the current account nonce counter. Defaults to `0` if unset.
pub(crate) fn get_account_nonce(env: &Env) -> u64 {
    get_shared(env, &ControllerKey::AccountNonce).unwrap_or(0u64)
}

/// Increments and returns the account nonce counter. Panics with `MathOverflow` if the
/// counter would overflow `u64`.
pub(crate) fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    set_shared(env, &ControllerKey::AccountNonce, &next);
    next
}

/// Reads the configured position limits. Panics with `PositionLimitsNotSet` if unset.
pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

/// Writes the position limits.
pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

/// Reads the minimum borrow collateral floor, in USD WAD scale. Falls back to
/// `constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD` if unset.
pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

/// Writes the minimum borrow collateral floor, in USD WAD scale.
pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

/// Reads the configuration for the position manager at `addr`. Returns `None` if none is stored.
pub(crate) fn get_position_manager(env: &Env, addr: &Address) -> Option<PositionManagerConfig> {
    get_shared(env, &ControllerKey::PositionManager(addr.clone()))
}

/// Writes the configuration for the position manager at `addr`. Removes the storage key
/// instead if `config.is_active` is `false`.
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
