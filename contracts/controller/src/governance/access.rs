//! Owner, pause, upgrade, and migration operations for the controller contract.
//!
//! Wraps `stellar_access` and `stellar_contract_utils` primitives (ownable, pausable,
//! upgradeable, role transfer) with controller-specific initialization and
//! instance-storage renewal.

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{assert_with_error, panic_with_error, Address, BytesN, Env};

#[cfg(test)]
use crate::Controller;
use crate::{config, constants::INITIAL_APP_VERSION, storage};
use common::constants::{DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD, POSITION_LIMIT_MAX};
use stellar_access::*;
use stellar_contract_utils::*;

/// Returns the current owner address. Panics with `GenericError::OwnerNotSet` if no owner is set.
fn owner_or_panic(env: &Env) -> Address {
    ownable::get_owner(env).unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet))
}

/// Sets `admin` as owner, sets default position limits and minimum borrow collateral,
/// stores the initial app version, and pauses the contract.
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

/// Renews the controller instance TTL, pauses the contract if not already paused,
/// and upgrades the contract to `new_wasm_hash`.
pub(crate) fn upgrade(env: &Env, new_wasm_hash: &BytesN<32>) {
    storage::renew_controller_instance(env);

    if !pausable::paused(env) {
        pausable::pause(env);
    }
    upgradeable::upgrade(env, new_wasm_hash);
}

/// Renews the controller instance TTL and stores `new_version` as the app version.
/// Panics with `GenericError::InternalError` if `new_version` is not greater than the
/// currently stored version.
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

/// Returns the stored app version, or `INITIAL_APP_VERSION` if none is stored.
pub(crate) fn get_app_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::AppVersion)
        .unwrap_or(INITIAL_APP_VERSION)
}

/// Renews the controller instance TTL and pauses the contract.
pub(crate) fn pause(env: &Env) {
    storage::renew_controller_instance(env);
    stellar_contract_utils::pausable::pause(env);
}

/// Renews the controller instance TTL and unpauses the contract.
pub(crate) fn unpause(env: &Env) {
    storage::renew_controller_instance(env);
    stellar_contract_utils::pausable::unpause(env);
}

/// Renews the controller instance TTL and starts a two-step ownership transfer to
/// `new_owner`, valid for acceptance until `live_until_ledger`. Emits an ownership
/// transfer event.
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

/// Renews the controller instance TTL and completes the pending ownership transfer,
/// making the caller the new owner.
pub(crate) fn accept_ownership(env: &Env) {
    storage::renew_controller_instance(env);
    ownable::accept_ownership(env);
}

#[cfg(test)]
#[path = "../../tests/governance/access.rs"]
mod tests;
