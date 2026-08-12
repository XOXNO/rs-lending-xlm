//! Read-only view functions exposed by the controller contract: account risk
//! metrics, position amounts, market index snapshots, and liquidation
//! estimates. None of these functions mutate storage.

use crate::constants::{MAX_VIEW_INPUTS, WAD};
use crate::context::Cache;
use common::collections::unique_hub_tokens;

use crate::risk;
use common::errors::GenericError;
use common::rates::{unscale_borrow, unscale_supply};
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, HubPayment,
    LiquidationEstimate, MarketIndexView, PaymentTuple, PriceStatus,
};
use soroban_sdk::{assert_with_error, Address, Env, Map, Vec};

#[cfg(not(feature = "certora"))]
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/views.rs"]

use crate::context::Cache;
use crate::positions::liquidation::execute_liquidation;
use crate::storage;

/// Panics if `values` exceeds `MAX_VIEW_INPUTS`.
fn require_view_inputs_bound<T>(env: &Env, values: &Vec<T>) {
    assert_with_error!(
        env,
        values.len() <= MAX_VIEW_INPUTS,
        GenericError::InvalidPayments
    );
}

/// Returns the account's health factor in WAD scale. Returns `i128::MAX` if
/// the account does not exist or has no debt.
pub(crate) fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = Cache::new_view(env);
    match storage::try_get_account(env, account_id) {
        Some(account) if !account.debt_free() => risk::calculate_account_risk_totals(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor
        .raw(),
        _ => i128::MAX,
    }
}

/// Returns true if the account's health factor is below one WAD.
pub(crate) fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    health_factor(env, account_id) < WAD
}

/// Converts the account's scaled supply position for `hub_asset` into asset
/// units at the current supply index, rounding half-up. Returns 0 if the
/// account has no supply position for `hub_asset`.
pub(crate) fn collateral_amount_for_hub_asset(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> i128 {
    let Some(position) = storage::try_get_supply_position(env, account_id, hub_asset) else {
        return 0;
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(hub_asset);
    let decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;

    // Half-up: scaled * supply_index → asset units (same as pool supplied_amount).
    unscale_supply(
        env,
        position.scaled_amount,
        market_index.supply_index,
        decimals,
    )
}

/// Converts the account's scaled debt position for `hub_asset` into asset
/// units at the current borrow index, rounding half-up. Returns 0 if the
/// account has no debt position for `hub_asset`.
pub(crate) fn borrow_amount_for_hub_asset(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> i128 {
    let Some(position) = storage::try_get_debt_position(env, account_id, hub_asset) else {
        return 0;
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(hub_asset);
    let decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;

    // Half-up: scaled * borrow_index → asset units (same as pool borrowed_amount).
    // Not ceil: view amounts are not liquidation close amounts.
    unscale_borrow(
        env,
        position.scaled_amount,
        market_index.borrow_index,
        decimals,
    )
}

/// Returns true if account metadata exists for `account_id`.
pub(crate) fn account_exists(env: &Env, account_id: u64) -> bool {
    storage::try_get_account_meta(env, account_id).is_some()
}

/// Returns the account's supply and debt position maps. Returns a pair of
/// empty maps if the account does not exist.
pub(crate) fn get_account_positions(
    env: &Env,
    account_id: u64,
) -> (
    Map<HubAssetKey, AccountPositionRaw>,
    Map<HubAssetKey, DebtPositionRaw>,
) {
    if !account_exists(env, account_id) {
        return (Map::new(env), Map::new(env));
    }

    (
        storage::get_supply_positions(env, account_id),
        storage::get_debt_positions(env, account_id),
    )
}

/// Returns the account's attributes, derived from its stored metadata.
pub(crate) fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

/// Returns the account's threshold-weighted collateral value in WAD scale,
/// the collateral value counted toward the health-factor numerator. Returns
/// 0 if the account does not exist.
pub(crate) fn liquidation_collateral_available(env: &Env, account_id: u64) -> i128 {
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);

    risk::calculate_account_risk_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    )
    .weighted_collateral
    .raw()
}

/// Returns the pool contract's address.
pub(crate) fn get_pool_address(env: &Env) -> Address {
    storage::get_pool(env)
}

/// Returns a `MarketIndexView` for each entry in `hub_assets`, combining the
/// current supply and borrow index with the asset's price status. Panics if
/// `hub_assets` exceeds `MAX_VIEW_INPUTS`.
pub(crate) fn get_all_market_indexes_detailed(
    env: &Env,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexView> {
    require_view_inputs_bound(env, hub_assets);
    let mut cache = Cache::new_view(env);
    cache.fetch_market_indexes(hub_assets);
    let assets = unique_hub_tokens(env, hub_assets);
    let statuses = if assets.is_empty() {
        Map::new(env)
    } else {
        crate::external::price_aggregator::fetch_prices_status(env, &assets)
    };
    let mut result = Vec::new(env);

    for hub_asset in hub_assets.iter() {
        let index = cache.cached_market_index(&hub_asset);
        let status = statuses
            .get(hub_asset.asset.clone())
            .unwrap_or_else(PriceStatus::unusable);

        result.push_back(MarketIndexView {
            asset: hub_asset.asset,
            supply_index: index.supply_index.raw(),
            borrow_index: index.borrow_index.raw(),
            price_wad: status.final_wad,
            primary_price_wad: status.primary_wad,
            anchor_price_wad: status.secondary_wad,
            price_timestamp: status.price_timestamp,
            stale: status.stale,
            deviation: status.deviation,
            valid: status.valid,
        });
    }

    result
}

/// Runs the liquidation calculation for the account against `debt_payments`
/// and returns the resulting seized collateral amounts, protocol fees,
/// refunds, maximum payment value, and bonus rate, without persisting any
/// state change. Panics if `debt_payments` exceeds `MAX_VIEW_INPUTS`, or if
/// the account is not eligible for liquidation.
pub(crate) fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
) -> LiquidationEstimate {
    require_view_inputs_bound(env, debt_payments);
    let mut cache = Cache::new_view(env);
    let account = storage::get_account(env, account_id);

    let result = execute_liquidation(env, &account, debt_payments, &mut cache);

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    for entry in result.seized {
        seized_collaterals.push_back(PaymentTuple {
            asset: entry.hub_asset.asset.clone(),
            amount: entry.amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset: entry.hub_asset.asset,
            amount: entry.protocol_fee,
        });
    }

    LiquidationEstimate {
        seized_collaterals,
        protocol_fees,
        refunds: result.refunds,
        max_payment_wad: result.max_debt_usd,
        bonus_rate_bps: result.bonus_bps,
    }
}

#[cfg(test)]
#[path = "../tests/views/mod.rs"]
mod tests;



pub(crate) fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    let borrow = storage::get_debt_positions(env, account_id);
    risk::calculate_account_risk_totals(env, &mut cache, &supply, &borrow)
        .total_collateral
        .raw()
}

pub(crate) fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    let supply = storage::get_supply_positions(env, account_id);
    risk::calculate_account_risk_totals(env, &mut cache, &supply, &borrow)
        .total_debt
        .raw()
}

pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    let _ = risk::restamp_listed_supply_ltv(&mut cache, &mut account);
    risk::calculate_account_risk_totals(env, &mut cache, &account.supply_positions, &account.borrow_positions)
        .ltv_collateral
        .raw()
}

