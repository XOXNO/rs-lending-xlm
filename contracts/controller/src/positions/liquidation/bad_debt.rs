//! Bad-debt cleanup apply helpers.

use common::types::{Account, AccountPositionType, PoolSeizeEntry};
use soroban_sdk::{Env, Vec};

use crate::context::Cache;
use crate::events::CleanBadDebtEvent;
use crate::external::pool::pool_seize_positions_call;
use crate::storage::{self, iter_debt_positions, iter_typed_positions};

/// Seizes all of an account's supply and debt shares, emits the cleanup event,
/// and removes the account.
pub(super) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        ctx.apply_withdraw_after_pool(env, &hub_asset, position.scaled_amount);
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        ctx.apply_repay_after_pool(env, &hub_asset, position.scaled_amount);
    }

    // One batched pool call covering every seized position, supplies first.
    let mut entries: Vec<PoolSeizeEntry> = Vec::new(env);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        entries.push_back(PoolSeizeEntry {
            hub_asset,
            side: AccountPositionType::Deposit,
            position: (&position).into(),
        });
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
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
