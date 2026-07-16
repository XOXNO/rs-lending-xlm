//! Strategy entry points: multiply, collateral/debt swaps, Blend migration, and flash loans.
//!
//! # Strategy façade checklist
//!
//! Every account-mutating strategy follows the same assembly order (flash loan
//! is the exception: no account, pool callback only):
//!
//! 1. **Auth** — `caller.require_auth()`
//! 2. **Reentrancy** — `require_not_flash_loaning` (and flash guard when needed)
//! 3. **Preflight** — hubs, amounts, mode, same-asset rules, listing eligibility
//! 4. **Account** — load/create + owner/delegate (migration/multiply may create)
//! 5. **Oracles** — `prefetch_strategy_oracles` for priced legs
//! 6. **Actions** — compose position bricks (`borrow_for_*`, withdraw/repay
//!    helpers, `process_deposit`, swaps)
//! 7. **Finalize** — `strategy_finalize` = post-pool risk gates + both sides
//!    persist (`remove_if_empty`) + events
//!
//! Debt-opening strategies (multiply, swap_debt, migrate) open debt only through
//! shared borrow gates. Debt-neutral strategies (swap_collateral,
//! repay_debt_with_collateral) still re-check HF at finalize.

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
use crate::oracle;
use crate::payments;
use crate::positions::{finalize_position_flow, PositionSides};
use crate::risk::{position_assets, validation};

/// Bulk-prefetch RedStone feeds for an account's positions plus strategy legs.
pub(crate) fn prefetch_strategy_oracles(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let env = cache.env().clone();
    let mut priced_assets = position_assets(&env, &account.supply_positions.keys());
    priced_assets.append(&position_assets(&env, &account.borrow_positions.keys()));
    for asset in extra_assets.iter() {
        payments::push_unique_address(&mut priced_assets, asset.clone());
    }
    oracle::prefetch_redstone_feeds(cache, &priced_assets);
}

/// Strategy tail: solvency on pool-returned state, persist supply+debt (or remove
/// empty account), emit position batch.
///
/// Borrow-cap enforcement stays at debt-opening entrypoints (multiply, swap_debt,
/// migrate), mirroring `process_borrow`.
pub(crate) fn strategy_finalize(env: &Env, account_id: u64, account: &Account, cache: &mut Cache) {
    validation::require_post_pool_risk_gates(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::BOTH, true);
}
