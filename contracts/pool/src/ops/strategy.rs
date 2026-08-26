//! Strategy open: mints debt like a borrow, optionally withholding a flash-style fee.
//!
//! Used when the hub opens a leveraged or strategy position that draws pool
//! liquidity. Fee (if charged) is booked as protocol revenue and never leaves
//! the pool as cash debit.

use common::errors::{FlashLoanError, GenericError};
use common::math::fp::{Bps, Ray};
use common::types::{PoolAction, PoolStrategyMutation};
use common::validation::require_nonneg_amount;

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::ops::borrow;
use crate::{events, interest, ops};

/// Intermediate result of strategy accounting before token transfer and events.
pub(crate) struct StrategyOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolStrategyMutation,
    /// Fee withheld when `charge_fee` is true.
    pub(crate) fee: i128,
}

/// Opens a strategy position and transfers net proceeds to `receiver`.
///
/// Emits a strategy-fee event (when fee > 0) and a market state event.
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

    events::emit_market_state(env, outcome.cache.snapshot());
    outcome.mutation
}

/// Computes the fee, mints debt for `action.amount`, and debits cash for the
/// net send amount.
///
/// The cash debit equals `amount - fee`; the fee remains in the pool and is
/// credited as protocol revenue via [`interest::add_protocol_revenue`].
pub(crate) fn accounting(env: &Env, action: PoolAction, charge_fee: bool) -> StrategyOutcome {
    let PoolAction {
        position,
        amount,
        hub_asset,
    } = action;
    require_nonneg_amount(env, amount);

    let mut cache = ops::renewed_market(env, &hub_asset);
    let fee = compute_fee(env, &cache, amount, charge_fee);

    let mut position = Ray::from(position.scaled_amount);
    borrow::mint_debt(env, &mut cache, &mut position, amount);

    let protocol_fee = Ray::from_asset(env, fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(&mut cache, protocol_fee);

    let amount_to_send = amount
        .checked_sub(fee)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    cache.debit_cash(amount_to_send);

    cache.commit();
    let mutation = cache.strategy_mutation(position, amount, amount_to_send);
    StrategyOutcome {
        cache,
        mutation,
        fee,
    }
}

/// Computes the strategy fee from `flashloan_fee` bps when `charge_fee` is true;
/// returns 0 otherwise (does not consult the market flash-loan enable flag).
///
/// Panics if the fee would exceed principal when charging.
fn compute_fee(env: &Env, cache: &Cache, amount: i128, charge_fee: bool) -> i128 {
    if !charge_fee {
        return 0;
    }
    let fee = Bps::from(i128::from(cache.params().flashloan_fee)).flash_loan_fee_on(env, amount);
    assert_with_error!(env, fee <= amount, FlashLoanError::StrategyFeeExceeds);
    fee
}
