//! One file per strategy entry point. Shared pieces: the helpers here
//! (`prefetch_strategy_prices`, `snapshot_balances`, `strategy_finalize`),
//! `legs.rs` for controller-custody position primitives (repay, withdraw,
//! withdraw-all, net-settle through the controller's own balance), and
//! `swap.rs` for the router trust boundary. Every strategy ends in
//! `strategy_finalize`: restamp LTV, post-pool risk gates, finalize.

#[cfg(test)]
#[path = "../../tests/strategies/mod.rs"]
mod tests;

pub(crate) mod flash_loan;
pub(crate) mod flash_position;
pub(crate) mod legs;
pub(crate) mod migrate_blend;
pub(crate) mod multiply;
pub(crate) mod repay_debt_with_collateral;
pub(crate) mod swap;
pub(crate) mod swap_collateral;
pub(crate) mod swap_debt;

pub(crate) use crate::positions::borrow_into_controller;
pub(crate) use legs::{
    execute_withdraw_all, net_settle_collateral_against_debt, repay_debt_from_controller,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
pub(crate) use swap::{swap_tokens, swap_tokens_or_passthrough};

use common::types::{Account, HubAssetKey, StrategySwap};
use soroban_sdk::{token, Address, Env, Map, Vec};

use crate::context::Cache;
use crate::events;
use crate::positions::{finalize_position_flow, get_supply_position_or_panic, PositionSides};
use crate::risk::{self, account_price_assets, validation};

/// Records `holder`'s current balance for each of `assets`, keyed by asset
/// address. Each distinct asset is read once; repeats in `assets` are skipped.
pub(crate) fn snapshot_balances(
    env: &Env,
    holder: &Address,
    assets: impl IntoIterator<Item = Address>,
) -> Map<Address, i128> {
    let mut snapshot = Map::new(env);
    for asset in assets {
        if snapshot.contains_key(asset.clone()) {
            continue;
        }
        let balance = token::Client::new(env, &asset).balance(holder);
        snapshot.set(asset, balance);
    }
    snapshot
}

/// Fetches oracle prices into `cache` for every asset in `account`'s supply
/// and borrow positions plus `extra_assets`.
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let assets = account_price_assets(cache.env(), account, extra_assets);
    cache.fetch_prices(&assets);
}

/// Restamps `account`'s listed collateral LTV, enforces post-trade solvency
/// and health-factor gates, and persists positions and spoke usage, emitting
/// the position batch event.
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Cache,
) {
    let _ = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::BOTH, true);
}

/// Withdraws `amount` of `from` collateral to the controller and swaps the
/// proceeds into `token_out`, passing through unswapped when `from.asset`
/// already equals `token_out`. Returns the amount of `token_out` received.
pub(crate) fn withdraw_and_swap_from_supply(
    env: &Env,
    account: &mut Account,
    cache: &mut Cache,
    caller: &Address,
    from: &HubAssetKey,
    amount: i128,
    token_out: &Address,
    swap: &StrategySwap,
    action: events::PositionAction,
) -> i128 {
    let supply_pos = get_supply_position_or_panic(env, account, from);

    let actual_withdrawn = withdraw_collateral_to_controller(
        env,
        account,
        cache,
        StrategyWithdraw {
            hub_asset: from,
            amount,
            position: &supply_pos,
            action,
        },
    );

    swap_tokens_or_passthrough(env, caller, &from.asset, actual_withdrawn, token_out, swap)
}
