//! Governance instance storage: controller address and role-revocation guards.

use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_THRESHOLD_INSTANCE, TTL_THRESHOLD_SHARED,
};
use common::errors::GenericError;

use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env, Symbol};

// ################## STORAGE KEYS ##################

#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    Controller,
    /// Scheduled role-revocation operation id -> `(target account, revoked
    /// role)`. Read by `cancel` to enforce the self-veto and CANCELLER-revocation
    /// veto-immunity guards.
    RoleRevocationTarget(BytesN<32>),
}

// ################## CHANGE STATE ##################

pub(crate) fn renew_governance_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

/// Records the target account and revoked role of a scheduled role revocation
/// for the `cancel` guards. The 180-day bump outlives the timelock delay
/// (≤14 days) and execution grace, so the record cannot archive out from under
/// a still-pending operation.
pub(crate) fn mark_role_revocation_target(
    env: &Env,
    operation_id: &BytesN<32>,
    account: &Address,
    role: &Symbol,
) {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage()
        .persistent()
        .set(&key, &(account.clone(), role.clone()));
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

// ################## QUERY STATE ##################

pub(crate) fn role_revocation_target(
    env: &Env,
    operation_id: &BytesN<32>,
) -> Option<(Address, Symbol)> {
    let key = GovernanceKey::RoleRevocationTarget(operation_id.clone());
    env.storage().persistent().get(&key).inspect(|_| {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    })
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

// ################## LOW-LEVEL HELPERS ##################
