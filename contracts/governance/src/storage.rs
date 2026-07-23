//! Instance storage for wired addresses, role-revocation cancel guards, and
//! Recovery-op marks.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::GenericError;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    /// Address of the governance-deployed price-aggregator (oracle authority).
    PriceAggregator,
    /// Scheduled role-revocation operation id -> target account. Read by
    /// `cancel` to enforce the self-veto guard.
    RoleRevocationTarget(BytesN<32>),
    /// Marks a scheduled operation id as a Recovery-tier council reset.
    RecoveryOp(BytesN<32>),
}

pub(crate) fn renew_governance_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Records the target account of a scheduled role revocation for the `cancel`
/// self-veto guard. The 180-day bump outlives the timelock delay (≤14 days)
/// and execution grace, so the record cannot archive out from under a
/// still-pending operation.
pub(crate) fn mark_role_revocation_target(
    env: &Env,
    operation_id: &BytesN<32>,
    account: &Address,
) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().set(&key, account);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

pub(crate) fn mark_recovery_op(env: &Env, operation_id: &BytesN<32>) {
    let key = GovernanceKey::RecoveryOp(operation_id.clone());
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// Clears recovery and role-revocation sidecars for an operation id.
pub(crate) fn clear_operation_sidecars(env: &Env, operation_id: &BytesN<32>) {
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RecoveryOp(operation_id.clone()));
    env.storage()
        .persistent()
        .remove(&GovernanceKey::RoleRevocationTarget(operation_id.clone()));
}

/// Non-renewing lookup: callers either cancel (then delete) or only gate on
/// the presence of a mark; bumping TTL before erase wastes budget.
pub(crate) fn role_revocation_target(env: &Env, operation_id: &BytesN<32>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RoleRevocationTarget(operation_id.clone()))
}

pub(crate) fn is_recovery_op(env: &Env, operation_id: &BytesN<32>) -> bool {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RecoveryOp(operation_id.clone()))
        .unwrap_or(false)
}

pub(crate) fn has_controller(env: &Env) -> bool {
    env.storage().instance().has(&GovernanceKey::Controller)
}

pub(crate) fn get_controller(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::Controller)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

pub(crate) fn set_controller(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::Controller, addr);
}

pub(crate) fn has_price_aggregator(env: &Env) -> bool {
    env.storage()
        .instance()
        .has(&GovernanceKey::PriceAggregator)
}

pub(crate) fn get_price_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&GovernanceKey::PriceAggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

pub(crate) fn set_price_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&GovernanceKey::PriceAggregator, addr);
}

#[cfg(test)]
#[path = "../tests/storage.rs"]
mod tests;
