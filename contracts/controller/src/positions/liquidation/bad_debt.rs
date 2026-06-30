//! Bad-debt cleanup apply helpers.

use common::types::{Account, AccountPositionType, HubAssetKey, ScaledPositionRaw};
use soroban_sdk::Env;

use crate::context::Cache;
use crate::events::CleanBadDebtEvent;
use crate::external::pool::pool_seize_position_call;
use crate::storage::{self, iter_debt_positions, iter_typed_positions};

pub(super) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_debt_usd: i128,
    total_collateral_usd: i128,
) {
    // dimensional: total_debt_usd/total_collateral_usd are Wad<USD>.raw.
    let ctx = cache.require_spoke_usage_context(account.spoke_id);
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        ctx.apply_withdraw_after_pool(env, &hub_asset, position.scaled_amount);
    }
    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        ctx.apply_repay_after_pool(env, &hub_asset, position.scaled_amount);
    }

    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        seize_pool_position(
            env,
            cache,
            AccountPositionType::Deposit,
            &hub_asset,
            (&position).into(),
        );
    }

    for (hub_asset, position) in iter_debt_positions(&account.borrow_positions) {
        seize_pool_position(
            env,
            cache,
            AccountPositionType::Borrow,
            &hub_asset,
            (&position).into(),
        );
    }

    cache.persist_spoke_usage();

    CleanBadDebtEvent {
        account_id,
        total_borrow_usd_wad: total_debt_usd,
        total_collateral_usd_wad: total_collateral_usd,
    }
    .publish(env);

    storage::remove_account_entry(env, account_id);
}

fn seize_pool_position(
    env: &Env,
    cache: &mut Cache,
    side: AccountPositionType,
    hub_asset: &HubAssetKey,
    position: ScaledPositionRaw,
) {
    let pool_addr = cache.cached_pool_address();
    pool_seize_position_call(env, &pool_addr, hub_asset, side, position);
}
