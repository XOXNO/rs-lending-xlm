//! Governance ownership, roles, and self-admin helpers.

use common::errors::GenericError;
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Symbol,
};
use stellar_access::{access_control, ownable};

use crate::{storage, timelock, Governance, GovernanceArgs, GovernanceClient};

pub(crate) const ORACLE_ROLE: &str = "ORACLE";
pub(crate) const PROPOSER_ROLE: &str = "PROPOSER";
pub(crate) const EXECUTOR_ROLE: &str = "EXECUTOR";
pub(crate) const CANCELLER_ROLE: &str = "CANCELLER";

pub(crate) fn default_operational_roles(env: &Env) -> [Symbol; 4] {
    [
        Symbol::new(env, ORACLE_ROLE),
        Symbol::new(env, PROPOSER_ROLE),
        Symbol::new(env, EXECUTOR_ROLE),
        Symbol::new(env, CANCELLER_ROLE),
    ]
}

pub(crate) fn require_known_governance_role(env: &Env, role: &Symbol) {
    for known in default_operational_roles(env) {
        if role == &known {
            return;
        }
    }
    assert_with_error!(env, false, GenericError::InvalidRole);
}

fn sync_pending_admin_transfer(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    let pending_admin_key = access_control::AccessControlStorageKey::PendingAdmin;

    if live_until_ledger == 0 {
        env.storage().temporary().remove(&pending_admin_key);
    } else {
        stellar_access::role_transfer::transfer_role(
            env,
            new_owner,
            &pending_admin_key,
            live_until_ledger,
        );
    }

    let current_admin = access_control::get_admin(env)
        .or_else(|| ownable::get_owner(env))
        .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));
    access_control::emit_admin_transfer_initiated(
        env,
        &current_admin,
        new_owner,
        live_until_ledger,
    );
}

fn sync_owner_access_control(env: &Env, previous_owner: &Address, new_owner: &Address) {
    let previous_admin = access_control::get_admin(env).unwrap_or_else(|| previous_owner.clone());

    env.storage()
        .instance()
        .set(&access_control::AccessControlStorageKey::Admin, new_owner);
    env.storage()
        .temporary()
        .remove(&access_control::AccessControlStorageKey::PendingAdmin);
    access_control::emit_admin_transfer_completed(env, &previous_admin, new_owner);

    for role in default_operational_roles(env) {
        access_control::grant_role_no_auth(env, new_owner, &role, new_owner);

        if previous_owner != new_owner
            && access_control::has_role(env, previous_owner, &role).is_some()
        {
            access_control::revoke_role_no_auth(env, previous_owner, &role, new_owner);
        }
    }
}

fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

pub(crate) fn apply_upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    storage::renew_governance_instance(env);
    stellar_contract_utils::upgradeable::upgrade(env, new_wasm_hash);
}

pub(crate) fn apply_transfer_ownership(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    storage::renew_governance_instance(env);
    let current_owner = owner_or_panic(env);

    stellar_access::role_transfer::transfer_role(
        env,
        new_owner,
        &ownable::OwnableStorageKey::PendingOwner,
        live_until_ledger,
    );
    ownable::emit_ownership_transfer(env, &current_owner, new_owner, live_until_ledger);
    sync_pending_admin_transfer(env, new_owner, live_until_ledger);
}

/// Disallows EXECUTOR/CANCELLER overlap, except owner recovery roles.
fn require_executor_canceller_separation(env: &Env, account: &Address, role: &Symbol) {
    let executor = Symbol::new(env, EXECUTOR_ROLE);
    let canceller = Symbol::new(env, CANCELLER_ROLE);
    let conflicting = if role == &executor {
        canceller
    } else if role == &canceller {
        executor
    } else {
        return;
    };
    assert_with_error!(
        env,
        access_control::has_role(env, account, &conflicting).is_none(),
        GenericError::InvalidRole
    );
}

pub(crate) fn apply_grant_role(env: &Env, account: &Address, role: &Symbol) {
    storage::renew_governance_instance(env);
    require_executor_canceller_separation(env, account, role);
    let owner = owner_or_panic(env);
    access_control::grant_role_no_auth(env, account, role, &owner);
}

pub(crate) fn apply_revoke_role(env: &Env, account: &Address, role: &Symbol) {
    storage::renew_governance_instance(env);
    // Reject no-op revokes.
    assert_with_error!(
        env,
        access_control::has_role(env, account, role).is_some(),
        GenericError::InvalidRole
    );
    let owner = owner_or_panic(env);
    access_control::revoke_role_no_auth(env, account, role, &owner);
}

#[contractimpl]
impl Governance {
    pub fn __constructor(env: Env, admin: Address, min_delay: u32) {
        ownable::set_owner(&env, &admin);
        access_control::set_admin(&env, &admin);

        for role in default_operational_roles(&env) {
            access_control::grant_role_no_auth(&env, &admin, &role, &admin);
        }

        timelock::require_nonzero_delay(&env, min_delay);
        stellar_governance::timelock::set_min_delay(&env, min_delay);
    }

    pub fn accept_ownership(env: Env) {
        storage::renew_governance_instance(&env);
        let previous_owner = owner_or_panic(&env);
        ownable::accept_ownership(&env);
        let new_owner = owner_or_panic(&env);
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }

    pub fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access_control::has_role(&env, &account, &role).is_some()
    }
}

#[cfg(test)]
#[path = "../tests/access.rs"]
mod tests;
