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
use crate::positions::supply::refresh_deferred_supply_risk_params;
use crate::positions::{enforce_post_pool_solvency, finalize_position_flow, PositionSides};
use crate::risk::{account_price_assets, valued_account};
use crate::storage;

/// Creates a strategy context that defers supply liquidation-tuple refreshes
/// to `strategy_finalize`, so intermediate legs cannot gate them.
pub(crate) fn strategy_context(env: &Env) -> Context {
    let mut cache = Context::new(env);
    cache.defer_supply_risk_refresh();
    cache
}

/// Caches account and extra-asset prices before strategy funding or callbacks.
/// Supply legs excluded from the account's valuation are not priced unless they
/// are also a debt or extra asset.
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Context,
    account_id: u64,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let valued = valued_account(cache.env(), account_id, account);
    let assets = account_price_assets(cache.env(), &valued, extra_assets);
    cache.fetch_prices(&assets);
}

/// Refreshes the liquidation tuples of touched supply legs against the merged
/// account, then listed collateral LTV, checks solvency, health and collateral
/// floor, persists positions and spoke usage, removes an empty account, and
/// emits the position batch naming the current NFT holder.
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Context,
) {
    // A `flash_position` receiver callback or a caller-selected router hop can
    // move the account NFT mid-call (GH-13); re-read the owner so the position
    // batch names the holder at the end of the call (CR-19).
    account.owner = storage::account_owner(env, account_id);
    cache.set_valued_account(account_id);
    refresh_deferred_supply_risk_params(env, account, cache);
    let _ = enforce_post_pool_solvency(env, cache, account_id, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::Both, true);
}
