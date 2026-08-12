
pub(crate) 

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env};

#[cfg(test)]
use crate::Controller;
use crate::{config, constants::INITIAL_APP_VERSION, storage};
use common::constants::{DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD, POSITION_LIMIT_MAX};
use stellar_access::*;
use stellar_contract_utils::*;

fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

pub(crate) fn init(env: &Env, admin: &Address) {
    ownable::set_owner(env, admin);
    ownable::emit_ownership_transfer_completed(env, admin);

    config::limits::set_position_limits(
        env,
        PositionLimits {
            max_supply_positions: POSITION_LIMIT_MAX,
            max_borrow_positions: POSITION_LIMIT_MAX,
        },
    );

    config::limits::set_min_borrow_collateral_usd(env, DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD);

    env.storage()
        .instance()
        .set(&ControllerKey::AppVersion, &INITIAL_APP_VERSION);

    pausable::pause(env);
}

pub(crate) fn upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    storage::renew_controller_instance(env);

    if !pausable::paused(env) {
        pausable::pause(env);
    }
    upgradeable::upgrade(env, new_wasm_hash);
}

pub(crate) fn migrate(env: &Env, new_version: u32) {
    storage::renew_controller_instance(env);
    let current_version: u32 = env
        .storage()
        .instance()
        .get(&ControllerKey::AppVersion)
        .unwrap_or(INITIAL_APP_VERSION);
    assert_with_error!(
        env,
        new_version > current_version,
        GenericError::InternalError
    );
    env.storage()
        .instance()
        .set(&ControllerKey::AppVersion, &new_version);
}

pub(crate) fn get_app_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::AppVersion)
        .unwrap_or(INITIAL_APP_VERSION)
}

pub(crate) fn pause(env: &Env) {
    storage::renew_controller_instance(env);
    stellar_contract_utils::pausable::pause(env);
}

pub(crate) fn unpause(env: &Env) {
    storage::renew_controller_instance(env);
    stellar_contract_utils::pausable::unpause(env);
}

pub(crate) fn transfer_ownership(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    storage::renew_controller_instance(env);
    // #[only_owner] already authenticated; low-level role_transfer does not re-auth.
    let current_owner = owner_or_panic(env);

    stellar_access::role_transfer::transfer_role(
        env,
        new_owner,
        &ownable::OwnableStorageKey::PendingOwner,
        live_until_ledger,
    );
    ownable::emit_ownership_transfer(env, &current_owner, new_owner, live_until_ledger);
}

pub(crate) fn accept_ownership(env: &Env) {
    storage::renew_controller_instance(env);
    ownable::accept_ownership(env);
}

#[cfg(test)]
#[path = "../tests/governance/access.rs"]
mod tests;
