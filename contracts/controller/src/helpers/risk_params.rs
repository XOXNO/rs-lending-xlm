//! Sync stored collateral risk params from the current effective market config.
//!
//! User-facing supply-side writes refresh LTV and liquidation bonus on every
//! touch. Liquidation threshold follows the same safety rule as keeper
//! propagation: loosening applies immediately; tightening requires HF >= 1.05.

use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw, AssetConfig};
use soroban_sdk::{Address, Env, Map};

use crate::cache::Cache;
use crate::emode;

use super::calculate_health_factor;

/// Minimum HF (1.05 WAD) required before lowering a position's liquidation threshold.
pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

/// Applies `effective_config` risk params to an in-flight collateral position.
pub fn refresh_supply_risk_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
) {
    position.loan_to_value = effective_config.loan_to_value;
    position.liquidation_bonus = effective_config.liquidation_bonus;
    apply_liquidation_threshold(
        env,
        cache,
        account,
        asset,
        position,
        effective_config.liquidation_threshold,
    );
}

/// Resolves e-mode-adjusted config for `asset`, then refreshes `position`.
pub fn refresh_supply_risk_params_for_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    position: &mut AccountPosition,
) {
    let e_mode = emode::active_e_mode_category(env, account.e_mode_category_id);
    let config = emode::effective_asset_config(env, account, asset, cache, &e_mode);
    refresh_supply_risk_params(env, cache, account, asset, position, &config);
}

fn apply_liquidation_threshold(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    position: &mut AccountPosition,
    new_lt: Bps,
) {
    let old_lt = position.liquidation_threshold;
    if new_lt.raw() >= old_lt.raw() {
        position.liquidation_threshold = new_lt;
        return;
    }

    if account.borrow_positions.is_empty() {
        position.liquidation_threshold = new_lt;
        return;
    }

    let supply_positions = supply_positions_with(account, asset, position, new_lt);
    let hf = calculate_health_factor(
        env,
        cache,
        &supply_positions,
        &account.borrow_positions,
    );
    if hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW) {
        position.liquidation_threshold = new_lt;
    }
}

fn supply_positions_with(
    account: &Account,
    asset: &Address,
    position: &AccountPosition,
    new_lt: Bps,
) -> Map<Address, AccountPositionRaw> {
    let mut supply_positions = account.supply_positions.clone();
    let mut hypothetical = *position;
    hypothetical.liquidation_threshold = new_lt;
    supply_positions.set(asset.clone(), (&hypothetical).into());
    supply_positions
}