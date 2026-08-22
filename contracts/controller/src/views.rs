use crate::constants::{MAX_VIEW_INPUTS, WAD};
use crate::context::Cache;
use common::collections::unique_hub_tokens;

use crate::risk;
use common::errors::GenericError;
use common::math::fp::Ray;
use common::rates::{unscale_borrow, unscale_supply};
use common::types::{
    AccountAttributes, AccountPositionRaw, DebtPositionRaw, HubAssetKey, HubPayment,
    LiquidationEstimate, MarketIndexView, PaymentTuple, PriceStatus, SeizeMode,
};
use soroban_sdk::{assert_with_error, Address, Env, Map, Vec};

use crate::positions::liquidation::{build_liquidation_plan, split_seized_shares};
use crate::storage;

/// Panics unless `values` has at most `MAX_VIEW_INPUTS` entries.
fn require_view_inputs_bound<T>(env: &Env, values: &Vec<T>) {
    assert_with_error!(
        env,
        values.len() <= MAX_VIEW_INPUTS,
        GenericError::InvalidPayments
    );
}

/// Returns the account's health factor in WAD, or `i128::MAX` if it has no
/// debt or does not exist.
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

/// Returns whether the account's health factor is below 1.0 (WAD), making it
/// eligible for liquidation.
pub(crate) fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    health_factor(env, account_id) < WAD
}

/// Returns the account's current supply position amount for `hub_asset` in
/// asset units, or 0 if it holds no such position.
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

/// Returns the account's current debt position amount for `hub_asset` in
/// asset units, or 0 if it holds no such position.
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

/// Returns whether an account with `account_id` has stored metadata.
pub(crate) fn account_exists(env: &Env, account_id: u64) -> bool {
    storage::try_get_account_meta(env, account_id).is_some()
}

/// Returns the account's raw supply and debt position maps, or two empty
/// maps if the account does not exist.
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

/// Returns the account's spoke id and position mode from its stored metadata.
pub(crate) fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

/// Returns the account's liquidation-threshold-weighted collateral value in
/// USD (WAD), or 0 if the account does not exist.
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

/// Returns the address of the pool contract registered with the controller.
pub(crate) fn get_pool_address(env: &Env) -> Address {
    storage::get_pool(env)
}

/// Returns the supply/borrow index and price status for each of
/// `hub_assets`, refreshing market indexes and current price statuses first.
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

/// Simulates liquidating the account with `debt_payments` under `seize_mode` and returns the
/// resulting seized collateral, protocol fees, refunds, and bonus rate, without persisting any
/// state changes.
///
/// The reported units follow the mode, so the estimate describes what execution would actually
/// move: `Transfer` reports asset units (what the pool would pay out and withhold), `Credit`
/// reports RAY-scaled supply shares (what would leave the liquidated account and what would be
/// reclassified as revenue). In credit mode the liquidator receives
/// `seized_collaterals - protocol_fees` shares.
pub(crate) fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
    seize_mode: SeizeMode,
) -> LiquidationEstimate {
    require_view_inputs_bound(env, debt_payments);
    let mut cache = Cache::new_view(env);
    let account = storage::get_account(env, account_id);

    let result = build_liquidation_plan(env, &account, debt_payments, &mut cache).into_result();

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    for entry in result.seized {
        let (seized_amount, fee_amount) = match seize_mode {
            SeizeMode::Transfer => (entry.amount, entry.protocol_fee),
            SeizeMode::Credit(_) => {
                let (fee_scaled, _) = split_seized_shares(
                    env,
                    Ray::from(entry.scaled_amount),
                    Ray::from(entry.bonus_scaled),
                    entry.liquidation_fees,
                );
                (entry.scaled_amount, fee_scaled.raw())
            }
        };
        seized_collaterals.push_back(PaymentTuple {
            asset: entry.hub_asset.asset.clone(),
            amount: seized_amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset: entry.hub_asset.asset,
            amount: fee_amount,
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

/// Returns the account's total supply position value in USD (WAD), or 0 if
/// it has no metadata or no supply positions.
pub(crate) fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_supply_usd(env, &mut cache, &supply).raw()
}

/// Returns the account's total debt position value in USD (WAD), or 0 if it
/// has no metadata or no debt positions.
pub(crate) fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_debt_usd(env, &mut cache, &borrow).raw()
}

/// Returns the account's LTV-weighted collateral value in USD (WAD), first
/// refreshing supply position LTVs to the currently listed spoke asset
/// configuration.
pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    let _ = risk::restamp_listed_supply_ltv(&mut cache, &mut account);
    risk::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
}

#[cfg(test)]
#[path = "../tests/views/mod.rs"]
mod tests;
