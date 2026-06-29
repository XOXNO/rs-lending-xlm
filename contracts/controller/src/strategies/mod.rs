//! Strategy flash-loan flows.
//!
//! Entrypoints compose position primitives and aggregator swaps. Swap output
//! is verified against router reports. Flows that open debt must reach
//! `strategy_finalize`; solvency gates run there.

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
    execute_withdraw_all, repay_debt_from_controller, withdraw_collateral_to_controller,
    StrategyRepay, StrategyWithdraw,
};
pub(crate) use swap::swap_tokens;

use controller_interface::types::Account;
use soroban_sdk::{Address, Env, Vec};

use crate::cache::Cache;
use crate::helpers::utils;
use crate::oracle;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::validation;

/// Bulk-prefetch RedStone feeds for an account's positions plus strategy legs.
pub(crate) fn prefetch_strategy_oracles(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let env = cache.env().clone();
    let mut priced_assets = crate::helpers::position_assets(&env, &account.supply_positions.keys());
    priced_assets.append(&crate::helpers::position_assets(
        &env,
        &account.borrow_positions.keys(),
    ));
    for asset in extra_assets.iter() {
        utils::push_unique_address(&mut priced_assets, asset.clone());
    }
    oracle::prefetch_redstone_feeds(cache, &priced_assets);
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
