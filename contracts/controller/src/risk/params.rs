use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw, AssetConfig, HubAssetKey};
use soroban_sdk::{Env, Map};

use crate::account::update_or_remove_supply_position;
use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::context::Cache;
use crate::risk::calculate_account_risk_totals;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RiskRefreshScope {
    LtvOnly,

    FullTuple,
}

/// Updates `position`'s loan-to-value from `effective_config`, and, when
/// `scope` is `FullTuple`, its gated liquidation parameters as well. Returns
/// whether the position changed.
pub(crate) fn refresh_supply_risk_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
    scope: RiskRefreshScope,
) -> bool {
    let before = *position;
    position.loan_to_value = effective_config.loan_to_value;
    if scope == RiskRefreshScope::FullTuple {
        apply_gated_liquidation_params(env, cache, account, hub_asset, position, effective_config);
    }
    *position != before
}

/// Restamps the loan-to-value of every still-listed supply position in
/// `account` to match its current spoke asset config, skipping positions
/// whose asset is no longer listed. Returns whether any position changed.
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

/// Applies `effective_config`'s liquidation threshold, bonus, and fees to
/// `position`. Skips the update when the change favors the liquidator and
/// the account carries debt, unless the account's hypothetical health
/// factor would still clear 1.05 WAD with the new threshold applied.
pub(crate) fn apply_gated_liquidation_params(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &mut AccountPosition,
    effective_config: &AssetConfig,
) {
    if favors_liquidator(position, effective_config)
        && !account.debt_free()
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

/// Reports whether `effective_config` gives a liquidator better terms than
/// `position`'s current liquidation threshold, bonus, or fees.
fn favors_liquidator(position: &AccountPosition, effective_config: &AssetConfig) -> bool {
    effective_config.liquidation_threshold.raw() < position.liquidation_threshold.raw()
        || effective_config.liquidation_bonus.raw() > position.liquidation_bonus.raw()
        || effective_config.liquidation_fees.raw() < position.liquidation_fees.raw()
}

/// Recomputes the account's health factor with `position`'s liquidation
/// threshold hypothetically replaced by `new_lt`, and returns whether it
/// stays at or above 1.05 WAD.
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

/// Returns a copy of `account`'s supply positions with `hub_asset`'s
/// liquidation threshold replaced by `new_lt`.
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
