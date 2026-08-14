use common::errors::SpokeError;
use common::types::SpokeConfig;
use common::validation::validate_liquidation_curve;
use soroban_sdk::{assert_with_error, Env};

use crate::{
    constants::{
        DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
        DEFAULT_LIQUIDATION_TARGET_HF_WAD,
    },
    events::{EventSpoke, UpdateSpokeEvent},
    storage,
};

/// Creates a new, non-deprecated spoke with the default liquidation curve
/// parameters and publishes an `UpdateSpokeEvent`. Returns the new spoke's
/// id.
pub(crate) fn add_spoke(env: &Env) -> u32 {
    let id = storage::increment_spoke_id(env);

    let spoke = SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
        hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
        liquidation_bonus_factor_bps: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    };
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);

    id
}

/// Marks spoke `id` as deprecated and publishes an `UpdateSpokeEvent`.
/// Panics if the spoke is already deprecated.
pub(crate) fn remove_spoke(env: &Env, id: u32) {
    let mut spoke = storage::get_spoke(env, id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);

    spoke.is_deprecated = true;
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);
}

/// Validates and sets the liquidation curve parameters for spoke `id`, then
/// publishes an `UpdateSpokeEvent`. Panics if the target health factor,
/// max-bonus health factor, or bonus factor falls outside its valid range.
pub(crate) fn set_spoke_liquidation_curve(
    env: &Env,
    id: u32,
    target_hf_wad: i128,
    hf_for_max_bonus_wad: i128,
    liquidation_bonus_factor_bps: u32,
) {
    validate_liquidation_curve(
        env,
        target_hf_wad,
        hf_for_max_bonus_wad,
        liquidation_bonus_factor_bps,
    );

    let mut spoke = storage::get_spoke(env, id);
    spoke.liquidation_target_hf_wad = target_hf_wad;
    spoke.hf_for_max_bonus_wad = hf_for_max_bonus_wad;
    spoke.liquidation_bonus_factor_bps = liquidation_bonus_factor_bps;
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);
}

use common::errors::GenericError;
use common::types::HubConfig;

use crate::events::CreateHubEvent;

/// Creates a new active hub and publishes a `CreateHubEvent`. Returns the
/// new hub's id.
pub(crate) fn create_hub(env: &Env) -> u32 {
    let id = storage::increment_hub_id(env);
    storage::set_hub(env, id, &HubConfig { is_active: true });

    CreateHubEvent { hub_id: id }.publish(env);

    id
}

/// Asserts that hub `hub_id` exists and is active. Panics otherwise.
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
    let active = storage::get_hub(env, hub_id).is_some_and(|hub| hub.is_active);
    assert_with_error!(env, active, GenericError::HubNotActive);
}
