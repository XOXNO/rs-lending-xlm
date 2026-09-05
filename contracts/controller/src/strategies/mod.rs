//! Strategy entry points with shared pricing and account finalization.
//! `legs` handles position operations; `payments` measures receipts and refunds;
//! `swap` enforces the router boundary. Account strategies refresh LTV and check
//! post-pool risk before persistence. Flash loans settle entirely in the pool
//! without opening or mutating an account.

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
    withdraw_and_swap_from_supply, StrategyRepay,
};
pub(crate) use swap::{swap_tokens, swap_tokens_or_passthrough};

use common::types::Account;
use soroban_sdk::{Address, Env, Vec};

use crate::context::Context;
use crate::positions::{enforce_post_pool_solvency, finalize_position_flow, PositionSides};
use crate::risk::account_price_assets;

/// Caches account and extra-asset prices before strategy funding or callbacks.
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Context,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let assets = account_price_assets(cache.env(), account, extra_assets);
    cache.fetch_prices(&assets);
}

/// Refreshes listed collateral LTV, checks solvency, health and collateral floor,
/// then persists positions and spoke usage and emits the position batch.
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Context,
) {
    let _ = enforce_post_pool_solvency(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::Both, true);
}
