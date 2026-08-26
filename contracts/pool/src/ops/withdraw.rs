//! Withdraw leg: burn supply shares, debit cash, transfer underlying out.
//!
//! Liquidation withdrawals may skip the max-utilization check and withhold a
//! protocol fee from the gross amount before paying the receiver.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{MarketStateSnapshot, PoolPositionMutation, PoolWithdrawEntry};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::{guards, interest, ops};

/// Intermediate result of withdraw accounting before token transfer.
pub(crate) struct WithdrawOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolPositionMutation,
    pub(crate) snapshot: MarketStateSnapshot,
    /// Asset units actually transferred (gross minus liquidation fee if any).
    pub(crate) net_transfer: i128,
}

/// Accrues interest, burns supply shares, debits cash, and transfers the net
/// proceeds to `receiver`.
///
/// Mutation `actual_amount` is the **gross** withdrawal; `net_transfer` is what
/// leaves the pool after any liquidation fee.
pub(crate) fn apply(
    env: &Env,
    receiver: &Address,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let outcome = accounting(env, is_liquidation, entry);

    outcome.cache.transfer_out(receiver, outcome.net_transfer);
    (outcome.mutation, outcome.snapshot)
}

/// Runs withdraw accounting without transferring tokens.
///
/// Resolves full or partial close, optionally withholds liquidation fee,
/// burns shares, and gates liquidity / utilization / solvency before debit.
pub(crate) fn accounting(
    env: &Env,
    is_liquidation: bool,
    entry: &PoolWithdrawEntry,
) -> WithdrawOutcome {
    require_nonneg_amount(env, entry.protocol_fee);
    let (mut cache, position) = ops::load_leg(env, &entry.action);

    let (burned, gross_amount) = resolve_close_or_partial(&cache, entry.action.amount, position);
    let net_transfer = withhold_liquidation_fee(
        env,
        &mut cache,
        gross_amount,
        is_liquidation,
        entry.protocol_fee,
    );

    let remaining = burn_position(env, &mut cache, position, burned);
    gate_and_debit(env, &mut cache, net_transfer, is_liquidation);

    let snapshot = cache.commit();
    let mutation = cache.position_mutation(remaining, gross_amount);
    WithdrawOutcome {
        cache,
        mutation,
        snapshot,
        net_transfer,
    }
}

/// Maps requested amount and position to shares burned and gross asset amount.
/// Panics if a nonzero gross amount would burn zero shares.
fn resolve_close_or_partial(cache: &Cache, amount: i128, position: Ray) -> (Ray, i128) {
    let (burned, gross_amount) = cache.resolve_withdrawal(amount, position);
    assert_with_error!(
        cache.env(),
        gross_amount == 0 || burned.raw() > 0,
        GenericError::WithdrawRoundsToZeroShares
    );
    (burned, gross_amount)
}

/// Burns `burned` from market supply and returns the user's remaining scaled position.
fn burn_position(env: &Env, cache: &mut Cache, position: Ray, burned: Ray) -> Ray {
    cache.burn_supply(burned);
    position.checked_sub(env, burned)
}

/// Enforces reserve, utilization, and solvency guards, then debits cash for
/// the net transfer. The utilization guard is skipped during liquidations.
fn gate_and_debit(env: &Env, cache: &mut Cache, net_transfer: i128, is_liquidation: bool) {
    cache.require_reserves(net_transfer);

    if !is_liquidation {
        guards::require_utilization_below_max(env, cache);
    }
    guards::require_solvent_withdraw_state(env, cache);
    cache.debit_cash(net_transfer);
}

/// Subtracts a liquidation protocol fee from the gross amount and books it as revenue.
///
/// Non-liquidation paths or zero fee return `gross_amount` unchanged. Panics
/// if the fee exceeds the gross withdrawal.
pub(crate) fn withhold_liquidation_fee(
    env: &Env,
    cache: &mut Cache,
    gross_amount: i128,
    is_liquidation: bool,
    protocol_fee: i128,
) -> i128 {
    if !is_liquidation || protocol_fee == 0 {
        return gross_amount;
    }
    assert_with_error!(
        env,
        gross_amount >= protocol_fee,
        CollateralError::WithdrawLessThanFee
    );

    let fee = Ray::from_asset(env, protocol_fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, fee);
    gross_amount
        .checked_sub(protocol_fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/withdraw.rs"]
mod tests;
