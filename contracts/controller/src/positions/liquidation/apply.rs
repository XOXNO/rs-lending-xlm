use crate::account;
use common::math::fp::{Ray, Wad};
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, DebtPosition, HubAssetKey,
    PoolAction, PoolSeizeEntry, PoolWithdrawEntry, RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use common::validation::expect_invariant;
use soroban_sdk::{assert_with_error, Address, Env, Vec};

use crate::account::update_or_remove_supply_position;
use crate::context::Cache;
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
/// along its protocol fee share. This is `SeizeMode::Transfer`: the pool burns the shares,
/// debits cash, and pays the liquidator in underlying.
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

/// Applies the seizure legs as a share credit — `SeizeMode::Credit`.
///
/// For each leg the liquidated account is debited by the whole scaled seizure `S`, the
/// receiving account is credited with `S - fee`, and `fee` is booked as pool revenue. No token
/// moves and the pool's `supplied` and `cash` are untouched, so the market's liquidity is
/// irrelevant to whether the liquidation can complete.
///
/// The fee is booked through `PoolSeizeEntry { side: Deposit }`, which reaches the pool's
/// `absorb_supply_as_revenue`: it *reclassifies* shares that already exist, raising `revenue`
/// alone. `SeizeMode::Transfer`'s `withhold_liquidation_fee` path would be wrong here — it
/// *mints* new revenue shares, correct only because the equivalent cash was withheld from an
/// outbound transfer. Nothing is withheld in credit mode, so minting would create a supplier
/// claim with no assets behind it.
///
/// Because only scaled amounts move, the result is independent of the supply index and so
/// immune to index drift between planning and application — a property `Transfer` lacks.
pub(crate) fn apply_liquidation_share_credit(
    env: &Env,
    account: &mut Account,
    receiver: &mut Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
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
        // The account-to-account half of the credit moves no spoke usage. Both accounts are
        // bound to the same spoke and hold the same hub asset, so the debit and the credit
        // cancel: `-S + (S - fee) = -fee`, leaving the protocol fee as the only usage delta.
        // Written as an asserted identity rather than an exit/entry pair on purpose — routing
        // the credit through `apply_spoke_entry` would put liquidation behind the spoke's
        // supply cap, and an account in a spoke sitting at its cap must stay liquidatable.
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

        // Debit the liquidated account by the whole seizure. `checked_sub` traps on a negative
        // result, so a seizure exceeding the position can never be booked. Risk parameters are
        // deliberately not refreshed, matching the liquidation withdraw leg.
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

        // The protocol's share is the only value that leaves the account system, so it is the
        // only spoke-usage movement — the same accounting bad-debt cleanup performs when it
        // absorbs a position into revenue.
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

/// Adds `scaled` supply shares of `hub_asset` to `receiver`.
///
/// An existing position keeps its own stamped risk tuple and simply grows; a new position is
/// stamped from the *current* listing, exactly as an ordinary supply would be. The liquidated
/// account's tuple never travels with the shares — importing it would let a liquidator move a
/// stale, more generous LTV or threshold onto an account of their choosing.
///
/// No entry gate runs: this is a seizure, not a new supply, so `is_collateralizable == false`
/// must not block it and the spoke supply cap must not either.
fn credit_supply_shares(
    env: &Env,
    receiver: &mut Account,
    hub_asset: &HubAssetKey,
    scaled: Ray,
    cache: &mut Cache,
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

/// Buffers the receiving account's position deltas for a completed share credit.
///
/// Called after the liquidated account's batch has been published, so the two accounts touched
/// by one credit-mode liquidation appear as two `UpdatePositionBatchEvent`s in a defined order:
/// liquidated account first, receiver second.
pub(crate) fn record_share_credit_updates(
    env: &Env,
    receiver: &Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Cache,
) {
    for entry in seized.iter() {
        let liquidator_scaled = credited_shares(env, &entry);
        if liquidator_scaled == Ray::ZERO {
            continue;
        }
        let position = get_supply_position_or_panic(env, receiver, &entry.hub_asset);
        let supply_index = Ray::from(entry.market_index.supply_index);
        cache.record_supply_position_update(
            // `LiqCredit`, not `LiqSeize`: this amount is net of the protocol
            // fee, while the liquidated account's seizure leg is gross of it.
            events::PositionAction::LiqCredit,
            &entry.hub_asset,
            entry.market_index.supply_index,
            liquidator_scaled
                .mul(env, supply_index)
                .to_asset_floor(entry.feed.asset_decimals),
            &position,
        );
    }
}

/// Enforces `max_supply_positions` on the receiving account against the hub assets a share
/// credit would open there.
///
/// The limit is enforced rather than bypassed because the liquidator chooses the receiver: a
/// revert is actionable (pass `SeizeMode::Credit(0)` for a fresh account), whereas letting the
/// bound be exceeded would grow accounts past the size the worst-case liquidation resource
/// budget is sized for.
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

/// Returns the scaled shares a share credit hands the receiver for `entry`, re-deriving the
/// split from the entry alone so every call site agrees.
fn credited_shares(env: &Env, entry: &SeizeEntry) -> Ray {
    let (_, liquidator_scaled) = math::split_seized_shares(
        env,
        Ray::from(entry.scaled_amount),
        Ray::from(entry.bonus_scaled),
        entry.liquidation_fees,
    );
    liquidator_scaled
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

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_seize_modes.rs"]
mod tests;
