//! Storage accessors for hub configuration and hub ID allocation.

use common::errors::GenericError;
use common::types::{ControllerKey, HubConfig};
use soroban_sdk::{panic_with_error, Env};

use crate::storage::{get_shared, set_shared};

/// Allocates and returns the next hub ID, starting from 1. Persists the updated counter
/// in instance storage. Panics with `MathOverflow` if the counter would overflow `u32`.
pub(crate) fn increment_hub_id(env: &Env) -> u32 {
    let key = ControllerKey::LastHubId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

/// Reads the configuration for `hub_id`. Returns `None` if no hub is stored at that ID.
pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    get_shared(env, &ControllerKey::Hub(hub_id))
}

/// Writes the configuration for `hub_id`.
pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    set_shared(env, &ControllerKey::Hub(hub_id), config);
}
