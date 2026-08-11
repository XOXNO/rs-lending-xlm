//! Net settlement: offsets a user's supply against their debt on the same market.
//!
//! Burns matched scaled supply and debt with no cash movement or token transfer.
//! Used by the hub to close mirrored legs atomically.

use common::errors::GenericError;
use common::math::fp::Ray;
use common::types::{
    MarketStateSnapshot, PoolNetSettleEntry, PoolNetSettleResult, ScaledPositionRaw,
};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, Env};

use crate::{guards, ops};

/// Settles up to `entry.amount` of debt using the user's supply position.
///
/// Cap is the ceiled debt value. Withdrawal and repay resolvers determine shares
/// burned; any gross that does not burn positive shares on both sides panics.
///
/// # Returns
///
/// Updated residual positions, market indexes, and settled asset amount, plus
/// a snapshot for the caller to emit.
pub(crate) fn apply(
    env: &Env,
    entry: &PoolNetSettleEntry,
) -> (PoolNetSettleResult, MarketStateSnapshot) {
    require_nonneg_amount(env, entry.amount);
    // Fail closed if the hub passes a negative scaled position (would invert burns).
    require_nonneg_amount(env, entry.supply_position.scaled_amount);
    require_nonneg_amount(env, entry.debt_position.scaled_amount);
    let mut cache = ops::synced_market(env, &entry.hub_asset);

    let supply_position = Ray::from(entry.supply_position.scaled_amount);
    let debt_position = Ray::from(entry.debt_position.scaled_amount);

    let max_debt = cache.unscale_borrow_ceil(debt_position);
    let capped_amount = entry.amount.min(max_debt);

    let (burned_supply, gross_amount) = cache.resolve_withdrawal(capped_amount, supply_position);
    let (burned_debt, overpayment) = cache.resolve_repay(gross_amount, debt_position);
    assert_with_error!(env, overpayment == 0, GenericError::InternalError);
    assert_with_error!(
        env,
        gross_amount == 0 || (burned_supply.raw() > 0 && burned_debt.raw() > 0),
        GenericError::NetSettleRoundsToZeroShares
    );

    cache.burn_supply(burned_supply);
    cache.burn_debt(burned_debt);

    guards::require_solvent_withdraw_state(env, &cache);

    let snapshot = cache.commit();
    let result = PoolNetSettleResult {
        supply_position: ScaledPositionRaw {
            scaled_amount: supply_position.checked_sub(env, burned_supply).raw(),
        },
        debt_position: ScaledPositionRaw {
            scaled_amount: debt_position.checked_sub(env, burned_debt).raw(),
        },
        market_index: cache.market_index(),
        settled_amount: gross_amount,
    };
    (result, snapshot)
}
