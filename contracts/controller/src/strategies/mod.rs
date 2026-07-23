//! Strategy entry points: multiply, collateral/debt swaps, Blend migration, flash loans.
//!
//! Account strategies: Auth → Reentrancy → Preflight → Account → Prices → Actions → Finalize.
//! Flash loan skips Account/Prices/Finalize (pool callback only).

pub(crate) mod flash_loan;
mod migrate_blend;
mod multiply;
pub(crate) mod positions;
mod repay_debt_with_collateral;
pub(crate) mod swap;
mod swap_collateral;
mod swap_debt;

pub(crate) use crate::positions::borrow::{borrow_for_migration, borrow_for_strategy};
pub(crate) use positions::{
    execute_withdraw_all, net_settle_collateral_against_debt, repay_debt_from_controller,
    withdraw_collateral_to_controller, StrategyRepay, StrategyWithdraw,
};
pub(crate) use swap::{swap_tokens, swap_tokens_or_passthrough};

use common::types::Account;
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::{self, account_price_assets, validation};

/// Bulk-fetch price-aggregator USD feeds for an account's positions plus strategy legs.
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let env = cache.env().clone();
    cache.fetch_prices(&account_price_assets(&env, account, extra_assets));
}

/// Safe-param restamp, post-pool HF, then persist both sides (remove if empty).
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Cache,
) {
    let _ = risk::restamp_listed_supply_safe_params(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::BOTH, true);
}
