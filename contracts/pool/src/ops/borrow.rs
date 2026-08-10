//! Borrow leg: mint debt shares, debit cash, transfer assets to the receiver.
//!
//! Accounting is split from token transfer so other ops (e.g. strategy) can
//! reuse [`mint_debt`] without paying out.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{MarketStateSnapshot, PoolBorrowEntry, PoolPositionMutation};
use common::validation::require_positive_amount;

use soroban_sdk::{assert_with_error, Address, Env};

use crate::cache::Cache;
use crate::{guards, ops};

/// Intermediate result of borrow accounting before token transfer.
pub(crate) struct BorrowOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolPositionMutation,
    pub(crate) snapshot: MarketStateSnapshot,
}

/// Accrue, mint debt, debit cash, commit, then transfer borrowed assets out.
///
/// # Returns
///
/// Position mutation and post-commit market snapshot for the batch event.
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

/// Run borrow accounting without transferring tokens.
///
/// Updates the user's scaled debt position by `entry.action.amount` in asset
/// units, debits market cash, and commits state.
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

/// Mint scaled debt for `amount` of underlying and enforce max utilization.
///
/// Requires positive amount and sufficient cash reserves. Panics if the scaled
/// mint rounds to zero shares.
pub(crate) fn mint_debt(env: &Env, cache: &mut Cache, position: &mut Ray, amount: i128) {
    require_positive_amount(env, amount);
    cache.require_reserves(amount);
    guards::require_liquidation_buffer(env, cache, amount);

    let minted = cache.calculate_scaled_borrow(amount);

    assert_with_error!(
        env,
        minted.raw() > 0,
        GenericError::BorrowRoundsToZeroShares
    );

    *position = position.checked_add(env, minted);
    cache.mint_debt(minted);
    guards::require_utilization_below_max(env, cache);
}
