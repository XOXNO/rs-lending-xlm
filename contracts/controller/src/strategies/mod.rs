//! Controller-level strategies: multi-step operations (flash loans, Blend
//! migration, multiply, collateral/debt swaps, repay-with-collateral) built
//! on top of the shared position legs in [`legs`], plus common helpers for
//! prefetching prices, finalizing a position after a strategy runs, and
//! withdrawing supply into the controller ahead of a swap.

pub(crate) mod flash_loan;
pub(crate) mod legs;
pub(crate) mod migrate_blend;
pub(crate) mod multiply;
pub(crate) mod repay_debt_with_collateral;
pub(crate) mod swap;
pub(crate) mod swap_collateral;
pub(crate) mod swap_debt;

pub(crate) use crate::positions::borrow::{borrow_for_migration, borrow_for_strategy};
pub(crate) use legs::{
    execute_withdraw_all, net_settle_collateral_against_debt, repay_debt_from_controller,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
pub(crate) use swap::{swap_tokens, swap_tokens_or_passthrough};

use common::types::{Account, HubAssetKey, StrategySwap};
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::positions::{finalize_position_flow, get_supply_position_or_panic, PositionSides};
use crate::risk::{self, account_price_assets, validation};

/// Fetches and caches oracle prices for every asset held by `account` plus
/// `extra_assets`.
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let env = cache.env().clone();
    cache.fetch_prices(&account_price_assets(&env, account, extra_assets));
}

/// Restamps listed-supply LTV, enforces post-pool risk gates, and finalizes
/// both position sides for `account_id`, persisting the account. Called after
/// a strategy has applied its position changes.
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

/// Withdraws `amount` of `from` out of the account's supply position into the
/// controller, then swaps the withdrawn amount into `token_out` per `swap`
/// (or passes it through unswapped). Refunds any unswapped remainder to
/// `caller`. Returns the resulting `token_out` amount. Does not deposit,
/// repay, or otherwise update positions with the swap output.
///
/// Order is fixed and intentional: the supply position is loaded and validated
/// first, then withdrawn into the controller, then swapped (or passed through).
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
