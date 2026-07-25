//! Refreshes supply risk params from spoke-aware market config.

use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw, AssetConfig, HubAssetKey};
use soroban_sdk::{Env, Map};

use crate::account::update_or_remove_supply_position;
use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::context::Cache;
use crate::risk::calculate_account_risk_totals;

/// Re-stamps a supply leg from the spoke listing.
///
/// LTV is unconditional: it bounds borrow capacity and never feeds liquidation.
/// Threshold, bonus, and fees move as one tuple behind the min-HF gate, since
/// restamping is permissionless on the supply path.
pub(crate) fn refresh_supply_risk_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
) {
    position.loan_to_value = effective_config.loan_to_value;
    apply_gated_liquidation_params(env, cache, account, hub_asset, position, effective_config);
}

/// Re-stamps a leg from the spoke listing when the asset is still listed.
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

/// Sets each supply leg's LTV from the spoke listing. The liquidation tuple is
/// gated and moves only through [`refresh_supply_risk_params`]. Returns true if
/// any leg was updated.
pub(crate) fn restamp_listed_supply_ltv(cache: &mut Cache, account: &mut Account) -> bool {
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
        if position.loan_to_value.raw() == config.loan_to_value.raw() {
            continue;
        }
        position.loan_to_value = config.loan_to_value;
        update_or_remove_supply_position(account, &hub_asset, &position);
        changed = true;
    }
    changed
}

/// Writes threshold, bonus, and fees together, keeping the three same-vintage.
/// A tuple that moves the liquidator's way is skipped unless the account is
/// debt-free or still clears the min HF under the new threshold. Both
/// permissionless restamp paths route through here.
pub(crate) fn apply_gated_liquidation_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
) {
    if favors_liquidator(position, effective_config)
        && !account.borrow_positions.is_empty()
        && !clears_min_hf(
            env,
            cache,
            account,
            hub_asset,
            position,
            effective_config.liquidation_threshold,
        )
    {
        return;
    }

    position.liquidation_threshold = effective_config.liquidation_threshold;
    position.liquidation_bonus = effective_config.liquidation_bonus;
    position.liquidation_fees = effective_config.liquidation_fees;
}

/// True when any leg of the tuple moves the liquidator's way: a lower
/// threshold, a higher bonus, or a lower fee — fees are carved out of the
/// bonus, so cutting them enlarges the liquidator's net take.
fn favors_liquidator(position: &AccountPosition, effective_config: &AssetConfig) -> bool {
    effective_config.liquidation_threshold.raw() < position.liquidation_threshold.raw()
        || effective_config.liquidation_bonus.raw() > position.liquidation_bonus.raw()
        || effective_config.liquidation_fees.raw() < position.liquidation_fees.raw()
}

fn clears_min_hf(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
    new_lt: Bps,
) -> bool {
    let supply_positions = supply_positions_with(account, hub_asset, position, new_lt);
    let hf =
        calculate_account_risk_totals(env, cache, &supply_positions, &account.borrow_positions)
            .health_factor;
    hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW)
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
