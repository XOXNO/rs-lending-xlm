use common::types::{Account, AccountPositionType, PoolSeizeEntry};
use soroban_sdk::{Env, Vec};

use crate::account::remove_account_and_burn_nft;
use crate::context::Context;
use crate::events::CleanBadDebtEvent;
use crate::external::pool::pool_seize_positions_call;
use crate::risk::AccountRiskTotals;
use crate::spoke_usage::UsageSide;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// Seizes remaining positions and releases spoke usage. Emits pre-cleanup WAD
/// USD totals, then removes the account and burns its NFT; emits no position deltas.
pub(crate) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Context,
    account_id: u64,
    account: &Account,
    totals: &AccountRiskTotals,
) {
    let mut entries: Vec<PoolSeizeEntry> = Vec::new(env);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        cache.apply_spoke_exit(
            account.spoke_id,
            UsageSide::Supply,
            &hub_asset,
            position.scaled_amount,
        );
        entries.push_back(PoolSeizeEntry {
            hub_asset,
            side: AccountPositionType::Deposit,
            position: (&position).into(),
        });
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        cache.apply_spoke_exit(
            account.spoke_id,
            UsageSide::Borrow,
            &hub_asset,
            position.scaled_amount,
        );
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
        total_borrow_usd_wad: totals.total_debt.raw(),
        total_collateral_usd_wad: totals.total_collateral.raw(),
    }
    .publish(env);

    remove_account_and_burn_nft(env, account_id);
}
