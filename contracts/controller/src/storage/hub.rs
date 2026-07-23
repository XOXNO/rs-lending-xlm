//! Hub config storage and hub-id allocation.

use common::errors::GenericError;
use common::types::{ControllerKey, HubConfig};
use soroban_sdk::{panic_with_error, Env};

use crate::storage::renew_protocol_shared_key;

/// Allocates and returns the next hub id, panicking on overflow.
pub(crate) fn increment_hub_id(env: &Env) -> u32 {
    let key = ControllerKey::LastHubId;
    let current: u32 = env.storage().instance().get(&key).unwrap_or(0);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage().instance().set(&key, &next);
    next
}

pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    let key = ControllerKey::Hub(hub_id);
    let hub: Option<HubConfig> = env.storage().persistent().get(&key);
    // Read-renewal policy: active hubs must not archive while markets use them.
    if hub.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    hub
}

pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    let key = ControllerKey::Hub(hub_id);
    env.storage().persistent().set(&key, config);
    renew_protocol_shared_key(env, &key);
}
