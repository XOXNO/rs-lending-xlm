//! Refreshes supply risk params from spoke-aware market config.

use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw, AssetConfig, HubAssetKey};
use soroban_sdk::{Env, Map};

use crate::context::Cache;
use crate::spoke;

use crate::risk::calculate_account_risk_totals;

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
    position.liquidation_fees = effective_config.liquidation_fees;
    apply_liquidation_threshold(
        env,
        cache,
        account,
        hub_asset,
        position,
        effective_config.liquidation_threshold,
    );
}

/// Refreshes position risk params while the spoke listing exists; a removed
/// spoke member keeps its stamped params. Deprecated spokes refresh normally:
/// their listings stay governance-managed via `edit_asset_in_spoke`.
pub fn refresh_supply_risk_params_for_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
) {
    if cache
        .cached_spoke_asset(account.spoke_id, hub_asset)
        .is_none()
    {
        return;
    }
    let config = spoke::effective_asset_config(cache, account.spoke_id, hub_asset);
    refresh_supply_risk_params(env, cache, account, hub_asset, position, &config);
}

/// Applies a new liquidation threshold, gating any decrease on a post-change
/// health factor at or above the min.
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

/// Returns a copy of the account's supply positions with `position` restamped at `new_lt`.
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
