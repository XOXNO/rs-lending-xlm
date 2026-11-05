//! Caller checks. The registrar admin holds every role bit.

use common::errors::GenericError;
use common::ttl::renew_instance;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::storage;
use crate::{CuratorError, ROLE_MASK};

fn authenticate(env: &Env, caller: &Address) {
    renew_instance(env);
    caller.require_auth();
}

pub(crate) fn require_owner(env: &Env, caller: &Address) {
    authenticate(env, caller);
    assert_with_error!(
        env,
        caller == &storage::owner(env),
        GenericError::NotAuthorized
    );
}

pub(crate) fn require_registrar(env: &Env, registrar: u32) -> Address {
    let admin = storage::registrar_admin(env, registrar)
        .unwrap_or_else(|| panic_with_error!(env, CuratorError::UnknownRegistrar));
    assert_with_error!(
        env,
        !storage::is_suspended(env, registrar),
        CuratorError::RegistrarSuspended
    );
    admin
}

pub(crate) fn require_admin(env: &Env, caller: &Address, registrar: u32) {
    authenticate(env, caller);
    let admin = require_registrar(env, registrar);
    assert_with_error!(env, caller == &admin, CuratorError::RoleMissing);
}

/// Admin, or any account holding a bit in `mask`.
pub(crate) fn require_role(env: &Env, caller: &Address, registrar: u32, mask: u32) {
    authenticate(env, caller);
    let admin = require_registrar(env, registrar);
    if caller == &admin {
        return;
    }
    let held = storage::role_bits(env, registrar, caller);
    assert_with_error!(env, held & mask != 0, CuratorError::RoleMissing);
}

pub(crate) fn require_role_bits(env: &Env, bits: u32) {
    assert_with_error!(env, bits & !ROLE_MASK == 0, CuratorError::RoleBits);
}

pub(crate) fn require_hub(env: &Env, hub_id: u32, registrar: u32) {
    let owner = storage::hub_registrar(env, hub_id)
        .unwrap_or_else(|| panic_with_error!(env, CuratorError::HubUnowned));
    assert_with_error!(env, owner == registrar, CuratorError::NotHubRegistrar);
}

pub(crate) fn require_spoke(env: &Env, spoke_id: u32, registrar: u32) {
    let owner = storage::spoke_registrar(env, spoke_id)
        .unwrap_or_else(|| panic_with_error!(env, CuratorError::SpokeUnowned));
    assert_with_error!(env, owner == registrar, CuratorError::NotSpokeRegistrar);
}
