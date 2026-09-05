use crate::account;
use common::math::fp::{Ray, Wad};
use common::math::fp_core::mul_div_floor;
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, DebtPosition, HubAssetKey,
    PoolAction, PoolSeizeEntry, PoolWithdrawEntry, RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use common::validation::expect_invariant;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use crate::account::update_or_remove_supply_position;
use crate::context::Context;
use crate::events;
use crate::external::pool::pool_seize_positions_call;
use crate::payments;
use crate::positions::liquidation::bad_debt;
use crate::positions::liquidation::curve::is_socializable_bad_debt;
use crate::positions::liquidation::math;
use crate::positions::{
    apply_repay_batch, apply_withdraw_batch, enforce_spoke_asset_flags,
    get_supply_position_or_panic, make_pool_action, FreezePolicy, WithdrawKind,
};
use crate::risk::validation;
use crate::risk::AccountRiskTotals;
use crate::spoke_usage::UsageSide;
use common::errors::{GenericError, SpokeError};

/// Repays from measured pool receipts. Returns WAD USD receipt value, capped
/// per planned leg and floor-scaled when a token under-delivers.
pub(crate) fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    cache: &mut Context,
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

        // Credit only tokens received by the pool.
        let received = payments::transfer_amount_measured(
            env,
            &entry.hub_asset.asset,
            liquidator,
            &pool_addr,
            entry.amount,
            GenericError::AmountMustBePositive,
        );

        // The measured transfer requires a positive amount, so division is safe.
        let leg_usd = if received >= entry.amount {
            Wad::from(entry.usd_wad)
        } else {
            Wad::from(mul_div_floor(env, entry.usd_wad, received, entry.amount))
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

/// Executes transfer-mode seizure: burns shares, debits pool cash, and pays
/// underlying to the liquidator after withholding the protocol fee.
pub(crate) fn apply_liquidation_seizures(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Context,
) {
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(env);
    for entry in seized.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::SeizureLeg,
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

/// Debits seized shares `S`, credits `S - fee`, and reclassifies `fee` as revenue.
/// No tokens move; pool supply and cash remain unchanged. Moving scaled shares
/// also avoids supply-index drift between planning and application.
///
/// Deposit-side seizure uses `absorb_supply_as_revenue` to reclassify existing
/// shares. Transfer-mode fee minting would create unbacked claims here because
/// credit mode withholds no outbound cash.
pub(crate) fn apply_liquidation_share_credit(
    env: &Env,
    account: &mut Account,
    receiver: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Context,
) {
    let mut fee_entries: Vec<PoolSeizeEntry> = Vec::new(env);

    for entry in seized.iter() {
        enforce_spoke_asset_flags(
            env,
            cache,
            account.spoke_id,
            &entry.hub_asset,
            FreezePolicy::SeizureLeg,
        );

        let seized_scaled = Ray::from(entry.scaled_amount);
        let (fee_scaled, liquidator_scaled) = math::split_seized_shares(
            env,
            seized_scaled,
            Ray::from(entry.bonus_scaled),
            entry.liquidation_fees,
        );
        // Same-spoke usage nets to `-S + (S - fee) = -fee`. An entry/exit pair
        // would incorrectly let the supply cap block liquidation.
        assert_with_error!(
            env,
            receiver.spoke_id == account.spoke_id,
            SpokeError::SpokeMismatch
        );
        assert_with_error!(
            env,
            seized_scaled.checked_sub(env, liquidator_scaled) == fee_scaled,
            GenericError::InternalError
        );

        // Checked subtraction prevents over-seizure; liquidation preserves risk stamps.
        let mut position = get_supply_position_or_panic(env, account, &entry.hub_asset);
        position.scaled_amount = position.scaled_amount.checked_sub(env, seized_scaled);
        update_or_remove_supply_position(account, &entry.hub_asset, &position);
        cache.record_supply_position_update(
            events::PositionAction::LiqSeize,
            &entry.hub_asset,
            entry.market_index.supply_index,
            entry.amount,
            &position,
        );

        credit_supply_shares(env, receiver, &entry.hub_asset, liquidator_scaled, cache);

        // Only the protocol fee leaves account supply and reduces spoke usage.
        if fee_scaled > Ray::ZERO {
            cache.apply_spoke_exit(
                account.spoke_id,
                UsageSide::Supply,
                &entry.hub_asset,
                fee_scaled,
            );
            fee_entries.push_back(PoolSeizeEntry {
                hub_asset: entry.hub_asset.clone(),
                side: AccountPositionType::Deposit,
                position: ScaledPositionRaw {
                    scaled_amount: fee_scaled.raw(),
                },
            });
        }
    }

    if !fee_entries.is_empty() {
        let pool_addr = cache.cached_pool_address();
        pool_seize_positions_call(env, &pool_addr, &fee_entries);
    }
}

/// Credits shares using the receiver's existing risk tuple or the current
/// listing for a new position. Never imports the liquidated account's potentially
/// more generous stamps. Seizure bypasses collateral permissions and supply caps.
fn credit_supply_shares(
    env: &Env,
    receiver: &mut Account,
    hub_asset: &HubAssetKey,
    scaled: Ray,
    cache: &mut Context,
) {
    if scaled == Ray::ZERO {
        return;
    }
    let mut position = match receiver.supply_positions.get(hub_asset.clone()) {
        Some(raw) => AccountPosition::from(&raw),
        None => {
            let config = cache.require_spoke_asset(receiver.spoke_id, hub_asset);
            receiver.get_or_create_supply_position(hub_asset, &config)
        }
    };
    position.scaled_amount = position.scaled_amount.checked_add(env, scaled);
    update_or_remove_supply_position(receiver, hub_asset, &position);
}

/// Buffers receiver deltas after publishing the liquidated account's batch,
/// keeping the two accounts' events separate and ordered.
pub(crate) fn record_share_credit_updates(
    env: &Env,
    receiver: &Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Context,
) {
    for entry in seized.iter() {
        let liquidator_scaled = credited_shares(env, &entry);
        if liquidator_scaled == Ray::ZERO {
            continue;
        }
        let position = get_supply_position_or_panic(env, receiver, &entry.hub_asset);
        let supply_index = Ray::from(entry.market_index.supply_index);
        cache.record_supply_position_update(
            // Receiver credit is net of fees; the liquidated account's seizure is gross.
            events::PositionAction::LiqCredit,
            &entry.hub_asset,
            entry.market_index.supply_index,
            liquidator_scaled
                .mul(env, supply_index)
                .to_asset_floor(env, entry.feed.asset_decimals),
            &position,
        );
    }
}

/// Enforces receiver position limits to preserve the liquidation resource bound.
/// The liquidator can choose `Credit(0)` if the existing receiver has no room.
pub(crate) fn require_credit_position_limit(
    env: &Env,
    receiver: &Account,
    seized: &Vec<SeizeEntry>,
) {
    let mut aggregated: AggregatedPayments = Vec::new(env);
    for entry in seized.iter() {
        if credited_shares(env, &entry) > Ray::ZERO {
            aggregated.push_back((entry.hub_asset.clone(), entry.amount));
        }
    }
    validation::validate_bulk_position_limits(
        env,
        receiver,
        AccountPositionType::Deposit,
        &aggregated,
    );
}

/// Derives net receiver shares with the shared fee-splitting rules.
fn credited_shares(env: &Env, entry: &SeizeEntry) -> Ray {
    let (_, liquidator_scaled) = math::split_seized_shares(
        env,
        Ray::from(entry.scaled_amount),
        Ray::from(entry.bonus_scaled),
        entry.liquidation_fees,
    );
    liquidator_scaled
}

/// Removes empty accounts or socializes insolvent debt under the collateral dust cap.
pub(crate) fn check_bad_debt_after_liquidation(
    env: &Env,
    cache: &mut Context,
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

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_seize_modes.rs"]
mod tests;
