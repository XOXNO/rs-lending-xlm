use common::errors::CollateralError;
use common::math::fp::{Bps, Wad};
use common::types::{Account, AccountPosition, AssetConfig, HubAssetKey};
use common::validation::expect_invariant;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use crate::account::update_or_remove_supply_position;
use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::context::Context;
use crate::risk::{calculate_account_risk_totals, validation};
use crate::{events, storage};

/// Which stored supply risk parameters to refresh.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum RiskRefreshScope {
    /// Refresh LTV without loading debt or changing liquidation terms.
    LtvOnly,

    /// Also refresh the liquidation tuple subject to its health-factor gate.
    FullTuple,
}

/// Refreshes LTV; `FullTuple` also applies gated liquidation parameters.
/// Returns whether the in-memory position changed.
pub(crate) fn refresh_supply_risk_params(
    env: &Env,
    cache: &mut Context,
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

/// Refreshes stored LTV snapshots in memory for listed supply assets, skipping
/// unlisted assets. Returns whether any position changed.
pub(crate) fn restamp_listed_supply_ltv(cache: &mut Context, account: &mut Account) -> bool {
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

/// Refreshes the liquidation threshold, bonus, and fees together. With debt,
/// changes favoring liquidators require hypothetical health factor >= 1.05.
pub(crate) fn apply_gated_liquidation_params(
    env: &Env,
    cache: &mut Context,
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

/// Detects a lower threshold or fee, or a higher bonus, than the stored tuple.
fn favors_liquidator(position: &AccountPosition, effective_config: &AssetConfig) -> bool {
    effective_config.liquidation_threshold.raw() < position.liquidation_threshold.raw()
        || effective_config.liquidation_bonus.raw() > position.liquidation_bonus.raw()
        || effective_config.liquidation_fees.raw() < position.liquidation_fees.raw()
}

/// Checks health factor >= 1.05 with this position's threshold replaced by `new_lt`.
fn clears_min_hf(
    env: &Env,
    cache: &mut Context,
    account: &Account,
    hub_asset: &HubAssetKey,
    position: &AccountPosition,
    new_lt: Bps,
) -> bool {
    let mut hypothetical = *position;
    hypothetical.liquidation_threshold = new_lt;
    let mut supply_positions = account.supply_positions.clone();
    supply_positions.set(hub_asset.clone(), (&hypothetical).into());
    let hf =
        calculate_account_risk_totals(env, cache, &supply_positions, &account.borrow_positions)
            .health_factor;
    hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW)
}

/// Allows any authenticated caller to refresh listed supply LTV snapshots.
/// `has_risks` also refreshes gated liquidation tuples and enforces health
/// factor >= 1.05. Rejects execution during a flash loan.
pub(crate) fn update_account_threshold(
    env: &Env,
    caller: Address,
    has_risks: bool,
    account_ids: Vec<u64>,
) {
    validation::require_authorized_caller(env, &caller);

    let scope = if has_risks {
        RiskRefreshScope::FullTuple
    } else {
        RiskRefreshScope::LtvOnly
    };

    let mut cache = Context::new(env);

    for account_id in account_ids {
        cache.reset_spoke_context();
        sync_account_thresholds(env, account_id, scope, &mut cache);
    }
}

/// Refreshes listed supply parameters to `scope`. Skips missing metadata,
/// empty supply, or unresolved NFT ownership. Only `FullTuple` loads debt
/// and enforces final health factor >= 1.05; writes only supply positions.
fn sync_account_thresholds(
    env: &Env,
    account_id: u64,
    scope: RiskRefreshScope,
    cache: &mut Context,
) {
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_supply_positions(env, account_id);
    if supply_positions.is_empty() {
        return;
    }

    // Fail closed: never update an account whose NFT owner cannot be resolved.
    let Some(owner) = storage::try_account_owner(env, account_id) else {
        return;
    };

    let full_tuple = scope == RiskRefreshScope::FullTuple;
    let borrow_positions = if full_tuple {
        storage::get_debt_positions(env, account_id)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    let mut account = storage::account_from_parts(owner, meta, supply_positions, borrow_positions);
    let assets = account.supply_positions.keys();

    let mut any_changed = false;
    for hub_asset in assets.iter() {
        let Some(spoke_config) = cache.cached_spoke_asset(account.spoke_id, &hub_asset) else {
            continue;
        };
        let asset_config = AssetConfig::from(&spoke_config);

        let raw = expect_invariant(env, account.supply_positions.get(hub_asset.clone()));
        let mut updated = AccountPosition::from(&raw);

        let changed = refresh_supply_risk_params(
            env,
            cache,
            &account,
            &hub_asset,
            &mut updated,
            &asset_config,
            scope,
        );
        if !changed {
            continue;
        }

        any_changed = true;
        update_or_remove_supply_position(&mut account, &hub_asset, &updated);

        let market_index = cache.cached_market_index(&hub_asset);
        cache.record_supply_position_update(
            events::PositionAction::ParamUpd,
            &hub_asset,
            market_index.supply_index.raw(),
            0,
            &updated,
        );
    }

    if any_changed {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }

    if full_tuple {
        let hf = calculate_account_risk_totals(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor;
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }

    cache.emit_position_batch(account_id, &account);
}

#[cfg(test)]
#[path = "../../tests/risk/params.rs"]
mod tests;
