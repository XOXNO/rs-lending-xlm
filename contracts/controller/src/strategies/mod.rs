//! Strategy and flash-loan flows.
//!
//! Orchestration entrypoints compose position primitives (`borrow`, `supply`,
//! `withdraw`, `repay`) with aggregator swaps. Swap output is never trusted
//! from router reports — see `swap.rs`.
//!
//! Standard levered pipeline:
//! auth → flash guard → account → cache(policy) → [preflight] → prefetch →
//! mutate (borrow/withdraw/swap/deposit/repay) → `strategy_finalize`.
//!
//! Invariant: flows that open debt must not return between the borrow step and
//! `strategy_finalize`; solvency gates run only at finalize.

pub(crate) mod flash_loan;
mod multiply;
pub(crate) mod positions;
mod repay_debt_with_collateral;
pub(crate) mod swap;
mod swap_collateral;
mod swap_debt;

pub(crate) use positions::{
    execute_withdraw_all, open_strategy_borrow, repay_debt_from_controller,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
pub(crate) use swap::swap_tokens;

use controller_interface::types::Account;
use soroban_sdk::{Address, Env, Vec};

use crate::cache::Cache;
use crate::helpers::utils;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::validation;

/// Bulk-prefetch RedStone feeds for an account's positions plus strategy legs.
pub(crate) fn prefetch_strategy_oracles(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let mut priced_assets: Vec<Address> = account.supply_positions.keys();
    priced_assets.append(&account.borrow_positions.keys());
    for asset in extra_assets.iter() {
        utils::push_unique_address(&mut priced_assets, asset.clone());
    }
    crate::oracle::prefetch_redstone_feeds(cache, &priced_assets);
}

/// Re-check solvency, persist both sides (or remove empty accounts), and emit
/// batched position/market events.
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Cache,
) {
    // Borrow-cap enforcement lives at the entrypoints that open debt (multiply,
    // swap_debt), mirroring `process_borrow`; debt-neutral strategies
    // (swap_collateral, repay_debt_with_collateral) skip it.
    validation::require_post_pool_risk_gates(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::BOTH, true);
}
