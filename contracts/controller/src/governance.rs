use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env};

#[cfg(test)]
use crate::Controller;
use crate::{config, constants::INITIAL_APP_VERSION, storage};
use common::constants::{DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD, POSITION_LIMIT_MAX};
use stellar_access::*;
use stellar_contract_utils::*;

/// Sets the owner, default borrow floor, maximum position limits, and initial
/// app version. Initializes the controller paused.
pub(crate) fn init(env: &Env, admin: &Address) {
    ownable::set_owner(env, admin);
    ownable::emit_ownership_transfer_completed(env, admin);

    config::registry::set_position_limits(
        env,
        PositionLimits {
            max_supply_positions: POSITION_LIMIT_MAX,
            max_borrow_positions: POSITION_LIMIT_MAX,
        },
    );

    config::registry::set_min_borrow_collateral_usd(env, DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD);

    env.storage()
        .instance()
        .set(&ControllerKey::AppVersion, &INITIAL_APP_VERSION);

    pausable::pause(env);
}

/// Pauses if needed, then schedules replacement of the controller Wasm.
pub(crate) fn upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    if !pausable::paused(env) {
        pausable::pause(env);
    }
    env.deployer()
        .update_current_contract_wasm(new_wasm_hash.clone());
}

/// Records a strictly greater app version; performs no storage transformation.
pub(crate) fn migrate(env: &Env, new_version: u32) {
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

/// Returns the stored app version, defaulting to `INITIAL_APP_VERSION`.
pub(crate) fn get_app_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::AppVersion)
        .unwrap_or(INITIAL_APP_VERSION)
}

/// Pauses the controller; fails if already paused.
pub(crate) fn pause(env: &Env) {
    pausable::pause(env);
}

/// Unpauses the controller; fails if already unpaused.
pub(crate) fn unpause(env: &Env) {
    pausable::unpause(env);
}

/// Starts a two-step ownership transfer to `new_owner`, acceptable until
/// ledger `live_until_ledger`.
pub(crate) fn transfer_ownership(env: &Env, new_owner: &Address, live_until_ledger: u32) {
    // The entrypoint authenticates the owner; low-level role transfer does not.
    let current_owner = ownable::get_owner(env)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));

    role_transfer::transfer_role(
        env,
        new_owner,
        &ownable::OwnableStorageKey::PendingOwner,
        live_until_ledger,
    );
    ownable::emit_ownership_transfer(env, &current_owner, new_owner, live_until_ledger);
}

/// Renews instance TTL and completes transfer with pending-owner authorization.
pub(crate) fn accept_ownership(env: &Env) {
    storage::renew_controller_instance(env);
    ownable::accept_ownership(env);
}

#[cfg(test)]
#[path = "../tests/governance/access.rs"]
mod tests;
