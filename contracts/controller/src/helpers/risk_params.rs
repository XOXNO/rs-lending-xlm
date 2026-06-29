//! Refreshes LTV, liquidation bonus and (gated) liquidation threshold on
//! supply positions from the effective (spoke-aware) market config.

use common::math::fp::{Bps, Wad};
use controller_interface::types::{
    Account, AccountPosition, AccountPositionRaw, AssetConfig, HubAssetKey,
};
use soroban_sdk::{Env, Map};

use crate::cache::Cache;
use crate::spoke;

use super::calculate_account_risk_totals;

/// Minimum HF (1.05 WAD) required before lowering a position's liquidation threshold.
pub const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

/// Applies `effective_config` risk params to an in-flight collateral position.
pub fn refresh_supply_risk_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
) {
    position.loan_to_value = effective_config.loan_to_value;
    position.liquidation_bonus = effective_config.liquidation_bonus;
    apply_liquidation_threshold(
        env,
        cache,
        account,
        hub_asset,
        position,
        effective_config.liquidation_threshold,
    );
}

/// Resolves the account-spoke risk config for `hub_asset`, then refreshes
/// `position`. A deprecated spoke or an asset removed from a named spoke keeps
/// the position's snapshotted params (no refresh), so view-modeled exits do not
/// reject on a governance change.
pub fn refresh_supply_risk_params_for_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
) {
    let active = matches!(cache.cached_spoke(account.spoke_id), Some(s) if !s.is_deprecated);
    if !active || cache.cached_spoke_asset(account.spoke_id, hub_asset).is_none() {
        return;
    }
    let config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);
    refresh_supply_risk_params(env, cache, account, hub_asset, position, &config);
}

fn apply_liquidation_threshold(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
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

    let supply_positions = supply_positions_with(account, hub_asset, position, new_lt);
    let hf = calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
        &supply_positions,
        &account.borrow_positions,
    )
    .health_factor;
    if hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW) {
        position.liquidation_threshold = new_lt;
    }
}

fn supply_positions_with(
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
    new_lt: Bps,
) -> Map<HubAssetKey, AccountPositionRaw> {
    let mut supply_positions = account.supply_positions.clone();
    let mut hypothetical = *position;
    hypothetical.liquidation_threshold = new_lt;
    supply_positions.set(hub_asset.clone(), (&hypothetical).into());
    supply_positions
}
