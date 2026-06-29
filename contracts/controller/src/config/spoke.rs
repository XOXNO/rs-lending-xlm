use common::errors::SpokeError;
use common::types::SpokeConfig;
use soroban_sdk::{assert_with_error, Env};

use crate::{
    events::{EventSpoke, UpdateSpokeEvent},
    storage,
};

pub fn add_spoke(env: &Env) -> u32 {
    let id = storage::increment_spoke_id(env);
    // The liquidation-curve fields default to zero; they stay inert until a
    // later phase reads them.
    let spoke = SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 0,
        hf_for_max_bonus_wad: 0,
        liquidation_bonus_factor_bps: 0,
    };
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);

    id
}

pub fn remove_spoke(env: &Env, id: u32) {
    let mut spoke = storage::get_spoke(env, id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    // Deprecation gates every spoke read (overlay, `active_spoke`, asset edits).
    // Discrete `SpokeAsset` keys are not enumerable, so member assets and their
    // market backlinks are left in place; the deprecation flag keeps them
    // unreachable.
    spoke.is_deprecated = true;
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);
}
