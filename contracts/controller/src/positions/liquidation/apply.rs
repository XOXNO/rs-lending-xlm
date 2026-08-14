use crate::account;
use common::math::fp::Wad;
use common::types::{
    Account, AccountPosition, DebtPosition, PoolAction, PoolWithdrawEntry, RepayEntry, SeizeEntry,
};
use common::validation::expect_invariant;
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::events;
use crate::payments;
use crate::positions::liquidation::bad_debt;
use crate::positions::liquidation::curve::is_socializable_bad_debt;
use crate::positions::{
    apply_repay_batch, apply_withdraw_batch, enforce_spoke_asset_flags, make_pool_action,
    FreezePolicy, WithdrawKind,
};
use crate::risk::AccountRiskTotals;
use common::errors::GenericError;

/// Pulls each `repaid` leg's tokens from `liquidator` into the pool and applies them as
/// repayments, floor-scaling a leg's USD value down when the pool received less than the
/// planned amount. Returns the total USD actually received.
pub(crate) fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Cache,
) -> Wad {
    let pool_addr = cache.cached_pool_address();
    let mut actions: Vec<PoolAction> = Vec::new(env);
    let mut received_usd = Wad::ZERO;
    for entry in repaid.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::AllowOnExit,
        );

        // Measure pool receipt (same as user supply/repay) so cash/debt books
        // never credit more than tokens actually received.
        let received = payments::transfer_amount_measured(
            env,
            &entry.hub_asset.asset,
            liquidator,
            &pool_addr,
            entry.amount,
            GenericError::AmountMustBePositive,
        );

        // Value that actually arrived for this leg. `transfer_amount_measured`
        // has already asserted `entry.amount > 0`, so the ratio is well defined.
        let leg_usd = if received >= entry.amount {
            Wad::from(entry.usd_wad)
        } else {
            Wad::from(common::math::fp_core::mul_div_floor(
                env,
                entry.usd_wad,
                received,
                entry.amount,
            ))
        };
        received_usd = received_usd.checked_add(env, leg_usd);

        let position: DebtPosition =
            (&expect_invariant(env, account.borrow_positions.get(entry.hub_asset.clone()))).into();
        actions.push_back(make_pool_action(&position, received, entry.hub_asset));
    }
    apply_repay_batch(
        env,
        account,
        liquidator,
        events::PositionAction::LiqRepay,
        &actions,
        cache,
    );
    received_usd
}

/// Withdraws each `seized` collateral leg to `liquidator` as a liquidation seizure, carrying
/// along its protocol fee share.
pub(crate) fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
) {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for entry in seized.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::AllowOnExit,
        );

        let position: AccountPosition =
            (&expect_invariant(env, account.supply_positions.get(entry.hub_asset.clone()))).into();
        entries.push_back(PoolWithdrawEntry {
            action: make_pool_action(&position, entry.amount, entry.hub_asset),
            protocol_fee: entry.protocol_fee,
        });
    }
    apply_withdraw_batch(
        env,
        account,
        liquidator,
        WithdrawKind::Liquidation,
        events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
}

/// After a liquidation, removes `account_id`'s entry if it has no debt left and no supply
/// positions either; if debt remains and exceeds the leftover collateral, with that collateral at
/// or below the dust threshold, socializes it as bad debt.
pub(crate) fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    totals: &AccountRiskTotals,
) {
    if account.borrow_positions.is_empty() {
        account::cleanup_account_if_empty(env, account, account_id);
        return;
    }

    if is_socializable_bad_debt(totals.total_debt, totals.total_collateral) {
        bad_debt::execute_bad_debt_cleanup(env, cache, account_id, account, totals);
    }
}
