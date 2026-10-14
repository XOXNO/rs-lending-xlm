use crate::account;
use common::constants::POSITION_LIMIT_MAX;
use common::math::fp::{Ray, Wad};
use common::math::fp_core::mul_div_floor;
use common::types::{
    Account, AccountPosition, AccountPositionType, AggregatedPayments, DebtPosition, HubAssetKey,
    HubPayment, LiquidationResult, PoolAction, PoolPositionMutation, PoolSeizeEntry,
    PoolWithdrawEntry, RepayEntry, ScaledPositionRaw, SeizeEntry,
};
use common::validation::expect_invariant;
use soroban_sdk::{assert_with_error, Address, Env, IntoVal, TryFromVal, Val, Vec};

use crate::account::update_or_remove_supply_position;
use crate::context::Context;
use crate::events;
use crate::external::pool::pool_seize_positions_call;
use crate::payments;
use crate::positions::liquidation::bad_debt;
use crate::positions::liquidation::math;
use crate::positions::{
    apply_repay_batch, apply_withdraw_batch, enforce_spoke_asset_flags, for_each_leg,
    get_supply_position_or_panic, make_pool_action, FreezePolicy, WithdrawKind,
};
use crate::risk::validation;
use crate::risk::{holds_valuation_excluded_leg, AccountRiskTotals};
use crate::spoke_usage::UsageSide;
use crate::storage;
use common::errors::{GenericError, OracleError, SpokeError};

/// Repays from measured pool receipts. Returns WAD USD receipt value, capped
/// per planned leg and floor-scaled when a token under-delivers.
///
/// With `offered` (a full-close plan), each leg pulls the merged offered amount
/// and the pool refunds what exceeds the debt; otherwise it pulls the planned amount.
/// Each pool mutation must report the decimals the plan priced the leg with.
pub(crate) fn apply_liquidation_repayments(
    env: &Env,
    liquidator: &Address,
    account: &mut Account,
    repaid: &Vec<RepayEntry>,
    offered: Option<&Vec<HubPayment>>,
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

        let pull = offered.map_or(entry.amount, |offers| {
            offered_amount(env, offers, &entry.hub_asset)
        });
        // Credit only tokens received by the pool.
        let received = payments::transfer_amount_measured(
            env,
            &entry.hub_asset.asset,
            liquidator,
            &pool_addr,
            pull,
            GenericError::AmountMustBePositive,
        );

        // Planned amounts are positive, so the division is safe.
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
    let results = apply_repay_batch(
        env,
        account,
        liquidator,
        events::PositionAction::LiqRepay,
        &actions,
        cache,
    );
    require_pool_decimals_match(env, repaid, &results, |entry| entry.feed.asset_decimals);
    received_usd
}

/// Requires each pool mutation's `asset_decimals` to equal the decimals the
/// plan priced that leg with. The plan converts USD to token units with the
/// oracle's decimals and the pool reads those units in its own, so a mismatch
/// would move a wrong power of ten; both values are in hand at the mutation.
fn require_pool_decimals_match<E>(
    env: &Env,
    entries: &Vec<E>,
    results: &Vec<PoolPositionMutation>,
    feed_decimals: fn(&E) -> u32,
) where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    for_each_leg(env, entries, results, |entry, result| {
        assert_with_error!(
            env,
            result.asset_decimals == feed_decimals(&entry),
            OracleError::InvalidOracleDecimals
        );
    });
}

/// Requires the pool's listed decimals to equal the feed decimals of every
/// planned leg, so an estimate fails where execution would. Execution checks
/// the same equality on the pool mutations themselves.
pub(crate) fn require_plan_decimals_match_pool(
    env: &Env,
    cache: &mut Context,
    result: &LiquidationResult,
) {
    for entry in result.repaid.iter() {
        require_leg_decimals_match_pool(env, cache, &entry.hub_asset, entry.feed.asset_decimals);
    }
    for entry in result.seized.iter() {
        require_leg_decimals_match_pool(env, cache, &entry.hub_asset, entry.feed.asset_decimals);
    }
}

/// Requires the pool's listed decimals for `hub_asset` to equal `feed_decimals`.
fn require_leg_decimals_match_pool(
    env: &Env,
    cache: &mut Context,
    hub_asset: &HubAssetKey,
    feed_decimals: u32,
) {
    let pool_decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;
    assert_with_error!(
        env,
        pool_decimals == feed_decimals,
        OracleError::InvalidOracleDecimals
    );
}

fn offered_amount(env: &Env, offers: &Vec<HubPayment>, hub_asset: &HubAssetKey) -> i128 {
    let (_, amount) = expect_invariant(env, offers.iter().find(|(key, _)| key == hub_asset));
    amount
}

/// Executes transfer-mode seizure: burns shares, debits pool cash, and pays
/// underlying to the liquidator after withholding the protocol fee. Each pool
/// mutation must report the decimals the plan sized the leg with.
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
    let results = apply_withdraw_batch(
        env,
        account,
        liquidator,
        WithdrawKind::Liquidation,
        events::PositionAction::LiqSeize,
        &entries,
        cache,
    );
    require_pool_decimals_match(env, seized, &results, |entry| entry.feed.asset_decimals);
}

/// Debits seized shares `S`, credits `S - fee`, and reclassifies `fee` as revenue.
/// No tokens move; pool supply and cash remain unchanged. Moving scaled shares
/// also avoids supply-index drift between planning and application.
///
/// The pool reclassifies the fee shares (`absorb_supply_as_revenue`); minting
/// them would create claims without backing cash. No pool mutation reports the
/// listed decimals here, so each seized leg's feed decimals are checked against
/// the pool's sync data instead, as the estimate does.
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

        require_leg_decimals_match_pool(env, cache, &entry.hub_asset, entry.feed.asset_decimals);

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
/// An empty receiver, such as a fresh `Credit(0)` account, skips the governed
/// limit: its slots are the liquidated account's legs, already bounded by the
/// limit that admitted them, so a lowered limit cannot leave an account
/// unliquidatable. Every receiver stays within `POSITION_LIMIT_MAX`, the bound
/// the per-account resource budget is sized for. The liquidator can choose
/// `Credit(0)` if the existing receiver has no room.
pub(crate) fn require_credit_position_limit(
    env: &Env,
    receiver: &Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Context,
) {
    let aggregated = credited_legs(env, seized);
    let max_allowed = if receiver.supply_positions.is_empty() {
        POSITION_LIMIT_MAX
    } else {
        storage::get_position_limits(env).max_supply_positions
    };
    validation::validate_position_count_within(
        env,
        receiver,
        AccountPositionType::Deposit,
        &aggregated,
        max_allowed.min(POSITION_LIMIT_MAX),
    );
    validation::require_whole_unit_isolation(env, cache, receiver, &aggregated);
}

/// Requires a current listing for every credited leg the receiver does not
/// hold yet, as `credit_supply_shares` does when it opens the position.
pub(crate) fn require_credit_listings(
    env: &Env,
    receiver: &Account,
    seized: &Vec<SeizeEntry>,
    cache: &mut Context,
) {
    for (hub_asset, _) in credited_legs(env, seized).iter() {
        if !receiver.supply_positions.contains_key(hub_asset.clone()) {
            let _ = cache.require_spoke_asset(receiver.spoke_id, &hub_asset);
        }
    }
}

/// Seized legs that credit the receiver a positive share amount.
fn credited_legs(env: &Env, seized: &Vec<SeizeEntry>) -> AggregatedPayments {
    let mut aggregated: AggregatedPayments = Vec::new(env);
    for entry in seized.iter() {
        if credited_shares(env, &entry) > Ray::ZERO {
            aggregated.push_back((entry.hub_asset.clone(), entry.amount));
        }
    }
    aggregated
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

/// Removes empty accounts or socializes insolvent debt that permissionless
/// cleanup admits (collateral at or below the dust cap, or a quote that repays
/// no debt unit), unless the account holds a supply leg excluded from valuation.
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

    // `totals` value an excluded leg at zero; cleanup would write off debt
    // against it and take it as revenue, so an account holding one stays open.
    // Checked first: the cleanup gate prices every supply leg of `account`,
    // and the liquidation never read the excluded leg's price.
    if !holds_valuation_excluded_leg(env, account_id, account)
        && bad_debt::admits_permissionless_cleanup(env, cache, account, totals)
    {
        bad_debt::execute_bad_debt_cleanup(env, cache, account_id, account, totals);
    }
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_seize_modes.rs"]
mod tests;
