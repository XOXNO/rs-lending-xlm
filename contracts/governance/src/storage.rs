//! Instance storage for wired addresses and operation sidecars.
//!
//! Sidecars mark Recovery ops and role-revocation targets so `cancel` can
//! enforce non-veto and self-veto rules. Instance holds controller and
//! price-aggregator addresses after one-shot deploy.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::GenericError;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    PriceAggregator,
    /// Scheduled role-revocation id → target account (self-veto guard).
    RoleRevocationTarget(BytesN<32>),
    /// Scheduled Recovery-tier id (non-cancellable).
    RecoveryOp(BytesN<32>),
}

/// Extends instance TTL. Every mutating entrypoint calls this.
pub(crate) fn renew_governance_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Records the revocation target for `cancel` self-veto checks.
///
/// Persistent TTL is 180 days so the mark outlives delay (≤14d) and grace.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().set(&key, account);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Flags `operation_id` as Recovery-tier (non-cancellable).
pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::RecoveryOp(operation_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Removes Recovery and role-revocation sidecars for `operation_id`.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RecoveryOp(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RoleRevocationTarget(operation_id.clone()));
}

/// Revocation target for `operation_id`, if marked. Does not extend TTL.
pub(crate) fn role_revocation_target(env: &Env, operation_id: &BytesN<32>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RoleRevocationTarget(operation_id.clone()))
}

/// Whether `operation_id` is marked Recovery-tier.
pub(crate) fn is_recovery_op(env: &Env, operation_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RecoveryOp(operation_id.clone()))
        .unwrap_or(false)
}

/// Whether the controller address is stored.
pub(crate) fn has_controller(env: &Env) -> bool {
    env.storage().instance().has(&GovernanceKey::Controller)
}

/// Stored controller address.
///
/// # Errors
/// * [`GenericError::PoolNotInitialized`] — controller not set.
pub(crate) fn get_controller(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::Controller)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Writes the controller address (one-shot in production deploy path).
pub(crate) fn set_controller(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::Controller, addr);
}

/// Whether the price-aggregator address is stored.
pub(crate) fn has_price_aggregator(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&GovernanceKey::PriceAggregator)
}

/// Stored price-aggregator address.
///
/// # Errors
/// * [`GenericError::AggregatorNotSet`] — aggregator not set.
pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Writes the price-aggregator address.
pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::PriceAggregator, addr);
}

#[cfg(test)]
#[path = "../tests/storage.rs"]
mod tests;
