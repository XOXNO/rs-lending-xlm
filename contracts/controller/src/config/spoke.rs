//! Spoke registry: creation stamps liquidation-curve defaults; removal
//! deprecates the spoke, which gates all subsequent spoke reads.

use common::errors::SpokeError;
use common::types::SpokeConfig;
use soroban_sdk::{assert_with_error, Env};

use crate::{
    constants::{
        DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
        DEFAULT_LIQUIDATION_TARGET_HF_WAD,
    },
    events::{EventSpoke, UpdateSpokeEvent},
    storage,
};

pub fn add_spoke(env: &Env) -> u32 {
    let id = storage::increment_spoke_id(env);
    // Liquidation-curve defaults are stamped at creation so storage and events
    // carry the effective values; liquidation reads them verbatim.
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

pub fn remove_spoke(env: &Env, id: u32) {
    let mut spoke = storage::get_spoke(env, id);
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    // Deprecation gates all spoke reads.
    spoke.is_deprecated = true;
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);
}
