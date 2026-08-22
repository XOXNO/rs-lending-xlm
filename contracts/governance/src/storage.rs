//! Persistent and instance storage access for the governance contract:
//! controller and price-aggregator addresses, and per-operation sidecar
//! state (role-revocation target, recovery-operation marker).

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::GenericError;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

/// Storage keys for governance contract state. `RoleRevocationTarget` and
/// `RecoveryOp` are keyed per timelock operation id.
#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    PriceAggregator,
    RoleRevocationTarget(BytesN<32>),
    RecoveryOp(BytesN<32>),
}

/// Records `account` as the role-revocation target for `operation_id` in
/// persistent storage and extends the entry's TTL.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().set(&key, account);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Marks `operation_id` as a recovery operation in persistent storage and
/// extends the entry's TTL.
pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::RecoveryOp(operation_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Removes the recovery-operation marker and role-revocation target entries
/// for `operation_id` from persistent storage.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RecoveryOp(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RoleRevocationTarget(operation_id.clone()));
}

/// Returns the account recorded as the role-revocation target for
/// `operation_id`, or `None` if no such entry exists.
pub(crate) fn role_revocation_target(env: &Env, operation_id: &BytesN<32>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RoleRevocationTarget(operation_id.clone()))
}

/// Returns whether `operation_id` is marked as a recovery operation.
/// Returns `false` if no marker entry exists.
pub(crate) fn is_recovery_op(env: &Env, operation_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RecoveryOp(operation_id.clone()))
        .unwrap_or(false)
}

/// Returns whether the controller address is set in instance storage.
pub(crate) fn has_controller(env: &Env) -> bool {
    env.storage().instance().has(&GovernanceKey::Controller)
}

/// Returns the controller address from instance storage. Panics with
/// `GenericError::PoolNotInitialized` if it is not set.
pub(crate) fn get_controller(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::Controller)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Stores `addr` as the controller address in instance storage.
pub(crate) fn set_controller(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::Controller, addr);
}

/// Returns whether the price aggregator address is set in instance storage.
pub(crate) fn has_price_aggregator(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&GovernanceKey::PriceAggregator)
}

/// Returns the price aggregator address from instance storage. Panics with
/// `GenericError::AggregatorNotSet` if it is not set.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Stores `addr` as the price aggregator address in instance storage.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::PriceAggregator, addr);
}

#[cfg(test)]
#[path = "../tests/storage.rs"]
mod tests;
