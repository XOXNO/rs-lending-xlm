use common::errors::GenericError;
use common::types::{ControllerKey, HubConfig};
use soroban_sdk::{panic_with_error, Env};

use super::protocol::{get_shared, set_shared};

/// Reads the last-issued hub ID from instance storage, increments it by one, stores the new value, and returns it. Panics with `MathOverflow` on overflow.
pub(crate) fn increment_hub_id(env: &Env) -> u32 {
    let key = ControllerKey::LastHubId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

/// Reads a hub's configuration from shared persistent storage, or `None` if it has not been set.
pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    get_shared(env, &ControllerKey::Hub(hub_id))
}

/// Writes a hub's configuration to shared persistent storage.
pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    set_shared(env, &ControllerKey::Hub(hub_id), config);
}
