//! Applies a built liquidation plan: debt repayments, collateral seizures, and
//! the post-liquidation bad-debt check.

use crate::account;
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, DebtPosition, PoolAction, PoolWithdrawEntry, RepayEntry, SeizeEntry,
};
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::external::sac::sac_transfer_call;
use crate::positions::liquidation::bad_debt;
use crate::positions::liquidation::math::is_socializable_bad_debt;
use crate::positions::{make_pool_action, repay, withdraw};
use crate::risk::validation;

/// Transfers each repayment from the liquidator and settles them in one pool call.
pub(super) fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Cache,
) {
    // Transfer each repayment in while building the actions for one bulk pool call.
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    for entry in repaid.iter() {
        // Debt lookup uses the full hub coordinate.
        sac_transfer_call(
            env,
            &entry.hub_asset.asset,
            liquidator,
            &pool_addr,
            &entry.amount,
        );

        let position: DebtPosition = (&validation::expect_invariant(
            env,
            account.borrow_positions.get(entry.hub_asset.clone()),
        ))
            .into();
        actions.push_back(make_pool_action(&position, entry.amount, entry.hub_asset));
    }
    repay::settle_repay_actions(
        env,
        account,
        liquidator,
        events::PositionAction::LiqRepay,
        &actions,
        cache,
    );
}

/// Builds every seizure entry and settles them in one bulk pool withdraw.
pub(super) fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
) {
    // Build all seizure entries for one bulk pool call.
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for entry in seized.iter() {
        // The supply-position lookup is keyed by the seized position's full hub key.
        let position: AccountPosition = (&validation::expect_invariant(
            env,
            account.supply_positions.get(entry.hub_asset.clone()),
        ))
            .into();
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, entry.amount, entry.hub_asset),
            protocol_fee: entry.protocol_fee,
        });
    }
    withdraw::settle_withdraw_entries(
        env,
        account,
        liquidator,
        events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
}

/// Cleans up an emptied account or socializes residual bad debt after liquidation.
pub(super) fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    total_collateral_usd: Wad,
    total_debt_usd: Wad,
) {
    if account.borrow_positions.is_empty() {
        account::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    if is_socializable_bad_debt(total_debt_usd, total_collateral_usd) {
        bad_debt::execute_bad_debt_cleanup(
            env,
            cache,
            account_id,
            account,
            total_debt_usd.raw(),
            total_collateral_usd.raw(),
        );
    }
}
