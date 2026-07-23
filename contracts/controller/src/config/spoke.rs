//! Spoke registry helpers: create (default liq curve), deprecate, curve update.

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

/// Registers a new spoke stamped with the default liquidation curve and returns its id.
pub(crate) fn add_spoke(env: &Env) -> u32 {
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

/// Deprecates a spoke, gating all subsequent spoke reads.
pub(crate) fn remove_spoke(env: &Env, id: u32) {
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

/// Overrides a spoke's liquidation curve (target HF, HF for max bonus, bonus
/// factor), replacing the defaults stamped at creation. `storage::get_spoke`
/// reverts `SpokeNotFound` for an unknown id.
pub(crate) fn set_spoke_liquidation_curve(
    env: &Env,
    id: u32,
    target_hf_wad: i128,
    hf_for_max_bonus_wad: i128,
    liquidation_bonus_factor_bps: u32,
) {
    // Re-validate at execution so a direct owner call cannot bypass bounds.
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
