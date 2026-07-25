//! Strategy leg: a borrow that withholds the market's flash-loan fee as
//! protocol revenue and pays out the remainder.

use common::errors::FlashLoanError;
use common::math::fp::{Bps, Ray};
use common::types::{PoolAction, PoolStrategyMutation};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use common::errors::GenericError;

use crate::cache::Cache;
use crate::ops::borrow;
use crate::{events, interest, ops};

/// Persisted result of the strategy accounting, before any token moves.
/// `fee` is the withheld flash-loan fee, already booked as protocol revenue.
pub(crate) struct StrategyOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolStrategyMutation,
    pub(crate) fee: i128,
}

/// Opens a strategy borrow and transfers `amount - fee` to `receiver`.
pub(crate) fn apply(
    env: &Env,
    receiver: &Address,
    action: PoolAction,
    charge_fee: bool,
) -> PoolStrategyMutation {
    let outcome = accounting(env, action, charge_fee);

    outcome
        .cache
        .transfer_out(receiver, outcome.mutation.amount_received);
    events::emit_strategy_fee(
        env,
        outcome.cache.hub_asset().hub_id,
        outcome.cache.hub_asset().asset.clone(),
        outcome.mutation.actual_amount,
        outcome.fee,
        outcome.mutation.amount_received,
    );
    outcome.mutation
}

/// Strategy accounting without the token transfer. `charge_fee = false`
/// (migration) borrows fee-free.
pub(crate) fn accounting(env: &Env, action: PoolAction, charge_fee: bool) -> StrategyOutcome {
    let PoolAction {
        position,
        amount,
        hub_asset,
    } = action;
    require_nonneg_amount(env, amount);

    let mut cache = ops::renewed_market(env, &hub_asset);

    let fee = if charge_fee {
        Bps::from(i128::from(cache.params().flashloan_fee)).flash_loan_fee_on(env, amount)
    } else {
        0
    };
    assert_with_error!(env, fee <= amount, FlashLoanError::StrategyFeeExceeds);

    let mut position = Ray::from(position.scaled_amount);
    borrow::mint_debt(env, &mut cache, &mut position, amount);

    let protocol_fee = Ray::from_asset(fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(&mut cache, protocol_fee);

    let amount_to_send = amount
        .checked_sub(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    // Debt is minted on the gross; only the net leaves. The withheld fee stays
    // as cash backing the revenue shares minted above.
    cache.debit_cash(amount_to_send);

    cache.commit();
    let mutation = cache.strategy_mutation(position, amount, amount_to_send);
    StrategyOutcome {
        cache,
        mutation,
        fee,
    }
}
