//! Supply leg: mints scaled shares against tokens the controller already
//! transferred into the pool.

use common::errors::GenericError;
use common::types::{MarketStateSnapshot, PoolPositionMutation, PoolSupplyEntry};

use soroban_sdk::{assert_with_error, Env};

use crate::{guards, ops};

/// Mints shares for `entry` and credits the cash the controller pre-transferred.
pub(crate) fn apply(
    env: &Env,
    entry: &PoolSupplyEntry,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let (mut cache, mut position) = ops::load_leg(env, &entry.action);
    let amount = entry.action.amount;

    // Gates on the pre-state deliberately: a supply adds equal cash and claim, so
    // checking after the mint would be vacuous and let deposits paper over a deficit.
    guards::require_backed_market(env, &cache);

    let minted = cache.calculate_scaled_supply(amount);
    assert_with_error!(
        env,
        amount == 0 || minted.raw() > 0,
        GenericError::SupplyRoundsToZeroShares
    );

    position = position.checked_add(env, minted);
    cache.mint_supply(minted);
    // Controller transferred Token(asset) `amount` into the pool before this call.
    cache.credit_cash(amount);

    let snapshot = cache.commit();
    (cache.position_mutation(position, amount), snapshot)
}
