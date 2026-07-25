//! Borrow leg: mints scaled debt, then pays the proceeds out.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{MarketStateSnapshot, PoolBorrowEntry, PoolPositionMutation};
use common::validation::require_positive_amount;

use soroban_sdk::{assert_with_error, Address, Env};

use crate::cache::Cache;
use crate::{guards, ops};

/// Persisted result of the borrow accounting, before any token moves.
pub(crate) struct BorrowOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolPositionMutation,
    pub(crate) snapshot: MarketStateSnapshot,
}

/// Mints debt for `entry` and transfers the proceeds to `receiver`.
pub(crate) fn apply(
    env: &Env,
    receiver: &Address,
    entry: &PoolBorrowEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let outcome = accounting(env, entry);

    outcome
        .cache
        .transfer_out(receiver, outcome.mutation.actual_amount);
    (outcome.mutation, outcome.snapshot)
}

/// Borrow accounting without the token transfer.
pub(crate) fn accounting(env: &Env, entry: &PoolBorrowEntry) -> BorrowOutcome {
    let (mut cache, mut position) = ops::load_leg(env, &entry.action);
    let amount = entry.action.amount;

    mint_debt(env, &mut cache, &mut position, amount);
    cache.debit_cash(amount);

    let snapshot = cache.commit();
    let mutation = cache.position_mutation(position, amount);
    BorrowOutcome {
        cache,
        mutation,
        snapshot,
    }
}

/// Mints scaled debt for `amount` against `position` and the market total.
///
/// Moves no cash — the caller debits, and the strategy leg debits only the net.
pub(crate) fn mint_debt(env: &Env, cache: &mut Cache, position: &mut Ray, amount: i128) {
    require_positive_amount(env, amount);
    cache.require_reserves(amount);

    let minted = cache.calculate_scaled_borrow(amount);
    // Unreachable by construction — ceil scaling makes any positive amount mint
    // positive shares. Kept so cash can never leave with zero debt recorded.
    assert_with_error!(
        env,
        minted.raw() > 0,
        GenericError::BorrowRoundsToZeroShares
    );

    *position = position.checked_add(env, minted);
    cache.mint_debt(minted);
    guards::require_utilization_below_max(env, cache);
}
