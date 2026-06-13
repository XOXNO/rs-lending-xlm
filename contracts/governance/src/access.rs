//! Ownership, the ORACLE role, and self-upgrade for the governance contract.
//!
//! Built on the `stellar_access` crate primitives. Governance owns the
//! ORACLE role, which gates the testing-only immediate oracle-configuration
//! forwarders; production oracle config is timelocked through the PROPOSER-gated
//! `propose_configure_market_oracle` / `propose_edit_oracle_tolerance` proposers.
//! Pause and position limits are absent by design: both are controller state and
//! stay behind the controller's own entrypoints.
//!
//! The entrypoints here (`upgrade`, `transfer_ownership`, `grant_role`,
//! `revoke_role`) administer the governance contract ITSELF and are owner-gated
//! and immediate. They cannot be timelocked: Soroban prohibits a contract from
//! invoking and self-authorizing itself, so a scheduled op cannot target
//! governance. Protocol-affecting controller admin IS timelocked, through the
//! typed `propose_*` proposers in `forward.rs`.

use common::errors::GenericError;
use soroban_sdk::{contractimpl, panic_with_error, Address, BytesN, Env, Symbol};
use stellar_access::{access_control, ownable};
use stellar_macros::only_owner;

use crate::{Governance, GovernanceArgs, GovernanceClient};

pub(crate) const ORACLE_ROLE: &str = "ORACLE";

/// Timelock roles. The crate leaves all role logic to the host; these gate the
/// `propose_*` / `execute` / `cancel` timelock entrypoints respectively.
pub(crate) const PROPOSER_ROLE: &str = "PROPOSER";
pub(crate) const EXECUTOR_ROLE: &str = "EXECUTOR";
pub(crate) const CANCELLER_ROLE: &str = "CANCELLER";

fn default_operational_roles(env: &Env) -> [Symbol; 1] {
    [Symbol::new(env, ORACLE_ROLE)]
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

#[contractimpl]
impl Governance {
    pub fn __constructor(env: Env, admin: Address, min_delay: u32) {
        ownable::set_owner(&env, &admin);
        access_control::set_admin(&env, &admin);
        access_control::grant_role_no_auth(&env, &admin, &Symbol::new(&env, ORACLE_ROLE), &admin);

        // Arm the initial proposer/executor/canceller set to `admin`. EXECUTOR
        // is granted so an explicit-executor execute path works, while open
        // execution (`executor: None`) stays available. `update_delay` is
        // owner-gated, so the delay setter rides the ownable admin, not a role.
        access_control::grant_role_no_auth(&env, &admin, &Symbol::new(&env, PROPOSER_ROLE), &admin);
        access_control::grant_role_no_auth(&env, &admin, &Symbol::new(&env, EXECUTOR_ROLE), &admin);
        access_control::grant_role_no_auth(
            &env,
            &admin,
            &Symbol::new(&env, CANCELLER_ROLE),
            &admin,
        );

        // Arm the timelock minimum delay; until this runs, `schedule` panics
        // `MinDelayNotSet`.
        stellar_governance::timelock::set_min_delay(&env, min_delay);
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        crate::storage::renew_governance_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    #[only_owner]
    pub fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        crate::storage::renew_governance_instance(&env);
        let current_owner = ownable::get_owner(&env).unwrap();

        stellar_access::role_transfer::transfer_role(
            &env,
            &new_owner,
            &ownable::OwnableStorageKey::PendingOwner,
            live_until_ledger,
        );
        ownable::emit_ownership_transfer(&env, &current_owner, &new_owner, live_until_ledger);
        sync_pending_admin_transfer(&env, &new_owner, live_until_ledger);
    }

    pub fn accept_ownership(env: Env) {
        crate::storage::renew_governance_instance(&env);
        let previous_owner = ownable::get_owner(&env).unwrap();
        ownable::accept_ownership(&env);
        let new_owner = ownable::get_owner(&env).unwrap();
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }

    #[only_owner]
    pub fn grant_role(env: Env, account: Address, role: Symbol) {
        crate::storage::renew_governance_instance(&env);
        let owner = ownable::get_owner(&env).unwrap();
        access_control::grant_role_no_auth(&env, &account, &role, &owner);
    }

    #[only_owner]
    pub fn revoke_role(env: Env, account: Address, role: Symbol) {
        crate::storage::renew_governance_instance(&env);
        let owner = ownable::get_owner(&env).unwrap();
        access_control::revoke_role_no_auth(&env, &account, &role, &owner);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Governance {
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
}
