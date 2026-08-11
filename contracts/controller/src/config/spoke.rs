//! Registers, deprecates, and configures spokes tracked by the controller.

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

/// Allocates the next spoke ID and stores a new spoke configuration with the default
/// liquidation curve. Publishes an `UpdateSpokeEvent` and returns the assigned ID.
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

/// Marks the spoke identified by `id` as deprecated and publishes an `UpdateSpokeEvent`.
/// Panics if the spoke is already deprecated, or if no spoke exists for `id`.
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

/// Validates and applies a new liquidation curve (target health factor, health factor
/// for maximum bonus, and bonus factor in basis points) to the spoke identified by `id`,
/// then publishes an `UpdateSpokeEvent`. Panics if the curve parameters are invalid or if
/// no spoke exists for `id`.
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
