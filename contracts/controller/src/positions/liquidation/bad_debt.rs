//! Residual bad-debt socialization.
//!
//! Zeroes spoke usage for all supply and debt shares, seizes them into the pool
//! in one call, emits `CleanBadDebtEvent`, and removes the account. No tokens
//! go to a liquidator — loss is socialized via pool indexes.
//!
//! Callers must already enforce eligibility (`is_socializable_bad_debt`) and auth.

use common::types::{Account, AccountPositionType, PoolSeizeEntry};
use soroban_sdk::{Env, Vec};

use crate::context::Cache;
use crate::events::CleanBadDebtEvent;
use crate::external::pool::pool_seize_positions_call;
use crate::storage::{self, iter_debt_positions, iter_typed_positions};

/// Seize all supply then debt shares, emit cleanup, remove the account.
pub(crate) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    // Usage first (full share drain), then one pool seize batch (supplies then debt).
    let mut entries: Vec<PoolSeizeEntry> = Vec::new(env);
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        ctx.apply_withdraw_after_pool(env, &hub_asset, position.scaled_amount);
        entries.push_back(PoolSeizeEntry {
            hub_asset,
            side: AccountPositionType::Deposit,
            position: (&position).into(),
        });
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        ctx.apply_repay_after_pool(env, &hub_asset, position.scaled_amount);
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
        total_borrow_usd_wad: total_debt_usd,
        total_collateral_usd_wad: total_collateral_usd,
    }
    .publish(env);

    storage::remove_account_entry(env, account_id);
}
