//! Governance instance storage: controller address and role-revocation guards.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::GenericError;
use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    /// Scheduled role-revocation operation id -> the account whose own removal
    /// it revokes. `cancel` blocks only that account from self-vetoing; every
    /// other canceller can still veto the operation.
    RoleRevocationTarget(BytesN<32>),
}

pub(crate) fn renew_governance_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Records the target account of a scheduled role revocation so `cancel` can
/// stop that account from vetoing its own removal. The 180-day bump outlives
/// the timelock delay (≤14 days) and execution grace, so the record cannot
/// archive out from under a still-pending operation.
pub(crate) fn mark_role_revocation_target(env: &Env, operation_id: &BytesN<32>, account: &Address) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().set(&key, account);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

pub(crate) fn role_revocation_target(env: &Env, operation_id: &BytesN<32>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GovernanceKey::RoleRevocationTarget(operation_id.clone()))
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
