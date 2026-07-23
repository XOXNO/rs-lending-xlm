//! Spoke liquidation-curve bounds. No external calls.

use common::validation::validate_liquidation_curve as common_validate_liquidation_curve;
use soroban_sdk::Env;

pub(crate) fn validate_liquidation_curve(
    env: &Env,
    target_hf_wad: i128,
    hf_for_max_bonus_wad: i128,
    bonus_factor_bps: u32,
) {
    common_validate_liquidation_curve(env, target_hf_wad, hf_for_max_bonus_wad, bonus_factor_bps);
}

#[cfg(test)]
#[path = "../../tests/validate/spoke.rs"]
mod tests;
