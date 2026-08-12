
pub(crate) mod flash_loan;
pub(crate) mod legs;
pub(crate) mod migrate_blend;
pub(crate) mod multiply;
pub(crate) mod repay_debt_with_collateral;
pub(crate) mod swap;
pub(crate) mod swap_collateral;
pub(crate) mod swap_debt;

pub(crate) use crate::positions::borrow::borrow_into_controller;
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

pub(crate) fn prefetch_strategy_prices(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let env = cache.env().clone();
    cache.fetch_prices(&account_price_assets(&env, account, extra_assets));
}

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
