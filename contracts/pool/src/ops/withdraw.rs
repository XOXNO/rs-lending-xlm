//! Withdraw leg: burns supply shares, takes the liquidation fee when one
//! applies, then pays the net amount out.
//!
//! Flow: resolve → withhold fee → burn → gate → debit → commit → transfer.

use common::errors::{CollateralError, GenericError};
use common::math::fp::Ray;
use common::types::{MarketStateSnapshot, PoolPositionMutation, PoolWithdrawEntry};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::{guards, interest, ops};

/// Persisted result of the withdraw accounting, before any token moves.
/// `mutation.actual_amount` is the gross withdrawal; `net_transfer` is what
/// actually leaves the pool once the liquidation fee is withheld.
pub(crate) struct WithdrawOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolPositionMutation,
    pub(crate) snapshot: MarketStateSnapshot,
    pub(crate) net_transfer: i128,
}

/// Burns shares for `entry` and transfers the net amount to `receiver`.
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

/// Withdraw accounting without the token transfer.
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
    // Mint the fee before burning: the share cap inside `add_protocol_revenue`
    // must see pre-burn supply, and `burn_supply` re-asserts backing over both.
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

/// Shares to burn and gross payout. Controller full-withdraw sentinel (amount ≥
/// half-up balance) closes the position and pays the floor value.
fn resolve_close_or_partial(cache: &Cache, amount: i128, position: Ray) -> (Ray, i128) {
    let (burned, gross_amount) = cache.resolve_withdrawal(amount, position);
    assert_with_error!(
        cache.env(),
        gross_amount == 0 || burned.raw() > 0,
        GenericError::WithdrawRoundsToZeroShares
    );
    (burned, gross_amount)
}

/// Burns `burned` from market supply and the caller's position.
fn burn_position(env: &Env, cache: &mut Cache, position: Ray, burned: Ray) -> Ray {
    cache.burn_supply(burned);
    position.checked_sub(env, burned)
}

/// Cash, utilization, and solvency gates, then debit tracked cash for the net.
fn gate_and_debit(env: &Env, cache: &mut Cache, net_transfer: i128, is_liquidation: bool) {
    // Gate the net, not the gross: the withheld fee never leaves the pool.
    cache.require_reserves(net_transfer);
    // Liquidations are exempt: a market cap must never block winding down an
    // underwater position, or utilization keeps climbing as the bad debt grows.
    if !is_liquidation {
        guards::require_utilization_below_max(env, cache);
    }
    guards::require_solvent_withdraw_state(env, cache);
    cache.debit_cash(net_transfer);
}

/// Withholds the liquidation fee from `gross_amount` and books it as protocol
/// revenue. The fee stays in the pool as cash, so supplier shares and the supply
/// index are untouched.
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

    let fee = Ray::from_asset(protocol_fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, fee);
    gross_amount
        .checked_sub(protocol_fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/withdraw.rs"]
mod tests;
