//! Ownership, roles (KEEPER / REVENUE / ORACLE), pause, and upgrade.
//!
//! Built on the `stellar_access` crate primitives. The three operational
//! roles are the only way to reach privileged-but-not-owner entrypoints
//! (index updates, revenue claims, oracle configuration). Pause is a
//! global circuit-breaker that still allows certain read and repay paths.

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env, Symbol,
};
use stellar_access::{access_control, ownable};
use stellar_macros::only_owner;

use crate::{storage, Controller, ControllerArgs, ControllerClient};

const INITIAL_APP_VERSION: u32 = 1;

pub(crate) const KEEPER_ROLE: &str = "KEEPER";
pub(crate) const REVENUE_ROLE: &str = "REVENUE";
pub(crate) const ORACLE_ROLE: &str = "ORACLE";

fn default_operational_roles(env: &Env) -> [Symbol; 3] {
    [
        Symbol::new(env, KEEPER_ROLE),
        Symbol::new(env, REVENUE_ROLE),
        Symbol::new(env, ORACLE_ROLE),
    ]
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
impl Controller {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);

        access_control::set_admin(&env, &admin);
        let keeper_role = Symbol::new(&env, KEEPER_ROLE);
        access_control::grant_role_no_auth(&env, &admin, &keeper_role, &admin);

        storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 10,
                max_borrow_positions: 10,
            },
        );

        env.storage()
            .instance()
            .set(&ControllerKey::AppVersion, &INITIAL_APP_VERSION);

        // Pause by default.
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        stellar_contract_utils::pausable::pause(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    // Post-upgrade migration entrypoint. Enforces strict version monotonicity.
    #[only_owner]
    pub fn migrate(env: Env, new_version: u32) {
        storage::renew_controller_instance(&env);
        let current_version: u32 = env
            .storage()
            .instance()
            .get(&ControllerKey::AppVersion)
            .unwrap_or(INITIAL_APP_VERSION);
        assert_with_error!(
            &env,
            new_version > current_version,
            GenericError::InternalError
        );
        env.storage()
            .instance()
            .set(&ControllerKey::AppVersion, &new_version);
    }

    pub fn app_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&ControllerKey::AppVersion)
            .unwrap_or(INITIAL_APP_VERSION)
    }

    #[only_owner]
    pub fn pause(env: Env) {
        storage::renew_controller_instance(&env);
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn unpause(env: Env) {
        storage::renew_controller_instance(&env);
        stellar_contract_utils::pausable::unpause(&env);
    }

    #[only_owner]
    pub fn grant_role(env: Env, account: Address, role: Symbol) {
        storage::renew_controller_instance(&env);
        // `#[only_owner]` already enforced owner auth; owner must exist here.
        let owner = ownable::get_owner(&env).unwrap();
        access_control::grant_role_no_auth(&env, &account, &role, &owner);
    }

    #[only_owner]
    pub fn revoke_role(env: Env, account: Address, role: Symbol) {
        storage::renew_controller_instance(&env);
        let owner = ownable::get_owner(&env).unwrap();
        access_control::revoke_role_no_auth(&env, &account, &role, &owner);
    }

    #[only_owner]
    pub fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        storage::renew_controller_instance(&env);
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
        storage::renew_controller_instance(&env);
        let previous_owner = ownable::get_owner(&env).unwrap();
        ownable::accept_ownership(&env);
        let new_owner = ownable::get_owner(&env).unwrap();
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use stellar_access::access_control::AccessControlStorageKey;
    use stellar_access::ownable::OwnableStorageKey;

    #[test]
    #[should_panic]
    fn test_sync_pending_admin_transfer_without_owner_or_admin() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(Controller, (admin,));
        let candidate = Address::generate(&env);
        env.as_contract(&contract_id, || {
            env.storage().instance().remove(&OwnableStorageKey::Owner);
            env.storage().instance().remove(&AccessControlStorageKey::Admin);
            sync_pending_admin_transfer(&env, &candidate, 100);
        });
    }
}

// `has_role` is test/`testing`-only. It needs its own fully cfg-gated
// `#[contractimpl]` so the method and its macro-generated dispatch strip
// together; gating inside the main impl leaves a dangling dispatch when the
// feature is off (E0425 under cross-crate feature unification).
#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl Controller {
    pub fn has_role(env: Env, account: Address, role: Symbol) -> bool {
        access_control::has_role(&env, &account, &role).is_some()
    }
}
