//! Protocol-global storage: instance-tier dependencies and risk floors, plus
//! persistent-tier entries (account-id nonce and the governance allowlists).

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits, PositionManagerConfig};

use soroban_sdk::{panic_with_error, Address, Env};

use crate::constants;
use crate::storage::renew_protocol_shared_key;

// Governance allowlists (Blend pools, position managers) are unbounded-set
// registries: one persistent key per entry, loaded on demand. Persistent (not
// instance) keeps them off the per-call instance envelope, so no cap counter is
// needed. `absent == not-approved`; each write extends the protocol-shared TTL.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    env.storage()
        .persistent()
        .get(&ControllerKey::BlendPoolAllowed(pool.clone()))
        .unwrap_or(false)
}

pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = ControllerKey::BlendPoolAllowed(pool.clone());
    if approved {
        env.storage().persistent().set(&key, &true);
        renew_protocol_shared_key(env, &key);
    } else {
        env.storage().persistent().remove(&key);
    }
}

pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

pub(crate) fn get_swap_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::SwapAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

pub(crate) fn set_swap_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::SwapAggregator, addr);
}

pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::PriceAggregator, addr);
}

pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

// Persistent, not instance: the nonce changes on every account creation, and
// an instance write rewrites (and re-rents) the whole instance envelope.
pub(crate) fn get_account_nonce(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&ControllerKey::AccountNonce)
        .unwrap_or(0u64)
}

/// Increments and returns the next account-id nonce, panicking on overflow.
pub(crate) fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let key = ControllerKey::AccountNonce;
    env.storage().persistent().set(&key, &next);
    renew_protocol_shared_key(env, &key);
    next
}

pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

/// Min borrow-collateral USD floor; defaults to constant when unset.
pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

/// Stores the minimum borrow-collateral USD floor.
pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

pub(crate) fn get_position_manager(env: &Env, addr: &Address) -> Option<PositionManagerConfig> {
    env.storage()
        .persistent()
        .get(&ControllerKey::PositionManager(addr.clone()))
}

/// Persistent registry of active managers; deactivation removes the entry
/// (absence == inactive for the delegate-auth check).
pub(crate) fn set_position_manager(env: &Env, addr: &Address, config: &PositionManagerConfig) {
    let key = ControllerKey::PositionManager(addr.clone());
    if config.is_active {
        env.storage().persistent().set(&key, config);
        renew_protocol_shared_key(env, &key);
    } else {
        env.storage().persistent().remove(&key);
    }
}

#[cfg(test)]
#[path = "../../tests/storage/protocol.rs"]
mod tests;
