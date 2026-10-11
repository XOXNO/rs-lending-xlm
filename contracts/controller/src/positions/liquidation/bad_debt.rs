use common::math::fp::Wad;
use common::rates::{position_value_floor, unscale_borrow_ceil};
use common::types::{Account, AccountPositionType, PoolSeizeEntry};
use soroban_sdk::{Env, Vec};

use super::curve::is_socializable_bad_debt;
use super::math::calculate_seizure_proportions;
use crate::account::remove_account_and_burn_nft;
use crate::context::Context;
use crate::events::CleanBadDebtEvent;
use crate::external::pool::pool_seize_positions_call;
use crate::risk::AccountRiskTotals;
use crate::spoke_usage::UsageSide;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// Admits permissionless cleanup (`clean_bad_debt` and the cleanup after a
/// liquidation) of an account whose debt exceeds its collateral when either
/// the collateral is at or below `BAD_DEBT_USD_THRESHOLD`
/// (`is_socializable_bad_debt`) or no liquidation can repay anything
/// (`quote_repays_no_debt_unit`). `totals` value the account's legs;
/// `cache` holds their prices and indexes.
pub(crate) fn admits_permissionless_cleanup(
    env: &Env,
    cache: &mut Context,
    account: &Account,
    totals: &AccountRiskTotals,
) -> bool {
    is_socializable_bad_debt(totals.total_debt, totals.total_collateral)
        || (totals.total_debt > totals.total_collateral
            && quote_repays_no_debt_unit(env, cache, account, totals))
}

/// Returns whether the collateral-backed quote of an insolvent account,
/// `floor(C / (1 + base))` (`curve::estimate_liquidation_amount`), is below
/// the smallest payment every debt leg accepts, so every liquidation offer is
/// trimmed to nothing and reverts `InvalidPayments`. A leg's smallest payment
/// is one native unit, valued as `calculate_repayment_amounts` values it: the
/// floor value of the whole debt when the debt rounds up to that one unit,
/// one unit at the price otherwise. A leg also accepts a payment when the
/// quote floors to at least one unit of it, as the insolvent trim keeps. With
/// a zero seizure proportion the insolvent quote does not apply and the
/// function returns `false`.
fn quote_repays_no_debt_unit(
    env: &Env,
    cache: &mut Context,
    account: &Account,
    totals: &AccountRiskTotals,
) -> bool {
    let (proportion_seized, bounds) = calculate_seizure_proportions(
        env,
        account,
        totals.total_collateral,
        totals.weighted_collateral,
        cache,
    );
    if proportion_seized <= Wad::ZERO {
        return false;
    }
    let one_plus_base = Wad::ONE.checked_add(env, bounds.base.to_wad(env));
    let quote = totals.total_collateral.div_floor(env, one_plus_base);
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let borrow_index = cache.cached_market_index(&hub_asset).borrow_index;
        let debt_units = unscale_borrow_ceil(
            env,
            position.scaled_amount,
            borrow_index,
            feed.asset_decimals,
        );
        let smallest_payment_usd = if debt_units <= 1 {
            position_value_floor(env, position.scaled_amount, borrow_index, feed.price)
        } else {
            feed.usd_value_wad(env, 1)
        };
        let kept_units = quote
            .div_floor(env, feed.price)
            .to_token_floor(env, feed.asset_decimals);
        if smallest_payment_usd <= quote || kept_units > 0 {
            return false;
        }
    }
    true
}

/// Seizes remaining positions and releases spoke usage. Emits pre-cleanup WAD
/// USD totals, then removes the account and burns its NFT; emits no position deltas.
pub(crate) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Context,
    account_id: u64,
    account: &Account,
    totals: &AccountRiskTotals,
) {
    let mut entries: Vec<PoolSeizeEntry> = Vec::new(env);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        cache.apply_spoke_exit(
            account.spoke_id,
            UsageSide::Supply,
            &hub_asset,
            position.scaled_amount,
        );
        entries.push_back(PoolSeizeEntry {
            hub_asset,
            side: AccountPositionType::Deposit,
            position: (&position).into(),
        });
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        cache.apply_spoke_exit(
            account.spoke_id,
            UsageSide::Borrow,
            &hub_asset,
            position.scaled_amount,
        );
        entries.push_back(PoolSeizeEntry {
            hub_asset,
            side: AccountPositionType::Borrow,
            position: (&position).into(),
        });
    }
    let pool_addr = cache.cached_pool_address();
    pool_seize_positions_call(env, &pool_addr, &entries);

    cache.persist_spoke_usage();

    CleanBadDebtEvent {
        account_id,
        total_borrow_usd_wad: totals.total_debt.raw(),
        total_collateral_usd_wad: totals.total_collateral.raw(),
    }
    .publish(env);

    remove_account_and_burn_nft(env, account_id);
}
