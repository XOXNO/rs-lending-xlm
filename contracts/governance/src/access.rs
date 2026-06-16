//! Ownership, governance roles, and self-admin apply helpers.
//!
//! Governance-self mutations (`upgrade`, delay changes, role grants/revokes,
//! ownership transfer initiation) are timelocked in `self_timelock.rs`.
//! This module holds the constructor, `accept_ownership`, shared apply
//! helpers, and the role allowlist.

use common::errors::GenericError;
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Symbol,
};
use stellar_access::{access_control, ownable};

use crate::{Governance, GovernanceArgs, GovernanceClient};

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
    crate::storage::renew_governance_instance(env);
    stellar_contract_utils::upgradeable::upgrade(env, new_wasm_hash);
}

pub(crate) fn apply_transfer_ownership(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    crate::storage::renew_governance_instance(env);
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

/// Separation of powers: a single delegated address must not hold BOTH the
/// EXECUTOR and CANCELLER timelock roles — whoever executes a scheduled
/// operation must not also be able to veto one. The owner is exempt: it holds
/// the full role set via the constructor / ownership sync, which call
/// `grant_role_no_auth` directly and bypass this path, so governance can never
/// deadlock (the owner can always both execute and cancel).
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
    crate::storage::renew_governance_instance(env);
    require_executor_canceller_separation(env, account, role);
    let owner = owner_or_panic(env);
    access_control::grant_role_no_auth(env, account, role, &owner);
}

pub(crate) fn apply_revoke_role(env: &Env, account: &Address, role: &Symbol) {
    crate::storage::renew_governance_instance(env);
    // Fail loud if the target does not hold the role: a silent no-op could let
    // an operator believe a privilege was removed when it never was.
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

        crate::timelock::require_nonzero_delay(&env, min_delay);
        stellar_governance::timelock::set_min_delay(&env, min_delay);
    }

    pub fn accept_ownership(env: Env) {
        crate::storage::renew_governance_instance(&env);
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
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    use crate::GovernanceClient;

    #[test]
    fn constructor_grants_oracle_role_to_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(
            Governance,
            (admin.clone(), crate::constants::TIMELOCK_MIN_DELAY_LEDGERS),
        );
        let client = GovernanceClient::new(&env, &contract_id);

        assert!(client.has_role(&admin, &Symbol::new(&env, ORACLE_ROLE)));
        env.as_contract(&contract_id, || {
            assert_eq!(ownable::get_owner(&env), Some(admin.clone()));
            assert_eq!(access_control::get_admin(&env), Some(admin));
        });
    }

    fn fresh_governance(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.register(
            Governance,
            (admin, crate::constants::TIMELOCK_MIN_DELAY_LEDGERS),
        )
    }

    // #7: a single delegate cannot hold both EXECUTOR and CANCELLER.
    #[test]
    #[should_panic]
    fn grant_role_enforces_executor_canceller_separation() {
        let env = Env::default();
        let id = fresh_governance(&env);
        let delegate = Address::generate(&env);
        env.as_contract(&id, || {
            apply_grant_role(&env, &delegate, &Symbol::new(&env, CANCELLER_ROLE));
            // Granting EXECUTOR to the same delegate must revert.
            apply_grant_role(&env, &delegate, &Symbol::new(&env, EXECUTOR_ROLE));
        });
    }

    // #7: separated EXECUTOR and CANCELLER delegates are allowed.
    #[test]
    fn grant_role_allows_separated_executor_and_canceller() {
        let env = Env::default();
        let id = fresh_governance(&env);
        let executor = Address::generate(&env);
        let canceller = Address::generate(&env);
        env.as_contract(&id, || {
            apply_grant_role(&env, &executor, &Symbol::new(&env, EXECUTOR_ROLE));
            apply_grant_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE));
            assert!(
                access_control::has_role(&env, &executor, &Symbol::new(&env, EXECUTOR_ROLE))
                    .is_some()
            );
            assert!(
                access_control::has_role(&env, &canceller, &Symbol::new(&env, CANCELLER_ROLE))
                    .is_some()
            );
        });
    }

    // #8: revoking a role the account does not hold reverts (no silent no-op).
    #[test]
    #[should_panic]
    fn revoke_role_rejects_unheld() {
        let env = Env::default();
        let id = fresh_governance(&env);
        let stranger = Address::generate(&env);
        env.as_contract(&id, || {
            apply_revoke_role(&env, &stranger, &Symbol::new(&env, ORACLE_ROLE));
        });
    }
}
