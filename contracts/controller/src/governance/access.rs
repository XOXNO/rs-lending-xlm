//! Owner, pause, and upgrade entrypoints. Pause blocks risk-increasing flows.

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, BytesN, Env};
use stellar_access::{access_control, ownable};
use stellar_macros::only_owner;

use common::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

use crate::{storage, Controller, ControllerArgs, ControllerClient};

const INITIAL_APP_VERSION: u32 = 1;

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
}

fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

#[contractimpl]
impl Controller {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);

        access_control::set_admin(&env, &admin);

        storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 10,
                max_borrow_positions: 10,
            },
        );

        storage::set_min_borrow_collateral_usd_wad(&env, DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD);

        env.storage()
            .instance()
            .set(&ControllerKey::AppVersion, &INITIAL_APP_VERSION);

        // New deployments start paused until the owner completes configuration and unpauses.
        stellar_contract_utils::pausable::pause(&env);
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        stellar_contract_utils::pausable::pause(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    // Bumps stored AppVersion; enforces strict monotonicity (no data rewrite).
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

    pub fn get_app_version(env: Env) -> u32 {
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
    pub fn transfer_ownership(env: Env, new_owner: Address, live_until_ledger: u32) {
        storage::renew_controller_instance(&env);
        let current_owner = owner_or_panic(&env);

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
        let previous_owner = owner_or_panic(&env);
        ownable::accept_ownership(&env);
        let new_owner = owner_or_panic(&env);
        sync_owner_access_control(&env, &previous_owner, &new_owner);
    }
}

#[cfg(test)]
#[path = "../../tests/governance/access.rs"]
mod tests;
