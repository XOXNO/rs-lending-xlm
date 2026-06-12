//! Instance storage for governance state.
//!
//! Holds the address of the controller this contract deploys and owns.

use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::errors::GenericError;
use soroban_sdk::{contracttype, panic_with_error, Address, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum GovernanceKey {
    /// Address of the controller deployed and owned by this contract.
    Controller,
}

pub(crate) fn renew_governance_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
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
