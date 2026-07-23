//! Refreshes supply risk params from spoke-aware market config.

use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw, AssetConfig, HubAssetKey};
use soroban_sdk::{Env, Map};

use crate::account::update_or_remove_supply_position;
use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::context::Cache;
use crate::risk::calculate_account_risk_totals;

pub(crate) fn refresh_supply_risk_params(
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

/// Re-stamps LTV, LT, bonus, and fees from the spoke listing when present.
pub(crate) fn refresh_supply_risk_params_for_asset(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
) {
    let Some(listed) = cache.cached_spoke_asset(account.spoke_id, hub_asset) else {
        return;
    };
    let config: AssetConfig = (&listed).into();
    refresh_supply_risk_params(env, cache, account, hub_asset, position, &config);
}

/// Sets each supply leg's LTV, liquidation bonus, and fees from the spoke listing.
/// Does not change liquidation threshold. Returns true if any leg was updated.
pub(crate) fn restamp_listed_supply_safe_params(
    cache: &mut Cache,
    account: &mut Account,
) -> bool {
    let mut changed = false;
    let keys = account.supply_positions.keys();
    for hub_asset in keys.iter() {
        let Some(listed) = cache.cached_spoke_asset(account.spoke_id, &hub_asset) else {
            continue;
        };
        let config: AssetConfig = (&listed).into();
        let Some(raw) = account.supply_positions.get(hub_asset.clone()) else {
            continue;
        };
        let mut position = AccountPosition::from(&raw);
        if position.loan_to_value.raw() == config.loan_to_value.raw()
            && position.liquidation_bonus.raw() == config.liquidation_bonus.raw()
            && position.liquidation_fees.raw() == config.liquidation_fees.raw()
        {
            continue;
        }
        position.loan_to_value = config.loan_to_value;
        position.liquidation_bonus = config.liquidation_bonus;
        position.liquidation_fees = config.liquidation_fees;
        update_or_remove_supply_position(account, &hub_asset, &position);
        changed = true;
    }
    changed
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

#[cfg(test)]
#[path = "../../tests/risk/params.rs"]
mod tests;
