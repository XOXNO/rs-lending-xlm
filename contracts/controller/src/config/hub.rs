use common::errors::GenericError;
use common::types::HubConfig;
use soroban_sdk::{assert_with_error, Env};

use crate::{events::CreateHubEvent, storage};

pub fn create_hub(env: &Env) -> u32 {
    let id = storage::increment_hub_id(env);
    storage::set_hub(env, id, &HubConfig { is_active: true });

    CreateHubEvent { hub_id: id }.publish(env);

    id
}

/// Requires an active hub registry entry; uncreated or inactive hubs revert.
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
    let active = storage::get_hub(env, hub_id).is_some_and(|hub| hub.is_active);
    assert_with_error!(env, active, GenericError::HubNotActive);
}
