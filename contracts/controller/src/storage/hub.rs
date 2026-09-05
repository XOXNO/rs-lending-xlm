use common::types::{ControllerKey, HubConfig};
use soroban_sdk::Env;

use super::protocol::{get_shared, increment_counter, set_shared};

/// Issues the next hub ID from the instance counter; fails on overflow.
pub(crate) fn increment_hub_id(env: &Env) -> u32 {
    increment_counter(env, &ControllerKey::LastHubId)
}

/// Returns shared persistent hub config, renewing TTL when present.
pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    get_shared(env, &ControllerKey::Hub(hub_id))
}

/// Stores hub config and renews shared persistent TTL.
pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    set_shared(env, &ControllerKey::Hub(hub_id), config);
}
