//! Repay leg: burn debt shares, credit cash, refund overpayment to the payer.
//!
//! The hub is expected to have transferred the repay amount into the pool
//! before calling. Overpayment (amount above outstanding debt) is returned.

use common::errors::GenericError;
use common::types::{MarketStateSnapshot, PoolAction, PoolPositionMutation};

use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::ops;

/// Intermediate result of repay accounting before overpayment transfer.
pub(crate) struct RepayOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolPositionMutation,
    pub(crate) snapshot: MarketStateSnapshot,
    /// Asset units above outstanding debt, refunded to the payer.
    pub(crate) overpayment: i128,
}

/// Accrue, burn debt, credit net repay, commit, refund overpayment.
///
/// # Returns
///
/// Mutation with `actual_amount` = net repay (excluding overpayment).
pub(crate) fn apply(
    env: &Env,
    payer: &Address,
    action: &PoolAction,
) -> (PoolPositionMutation, MarketStateSnapshot) {
    let outcome = accounting(env, action);

    outcome.cache.transfer_out(payer, outcome.overpayment);
    (outcome.mutation, outcome.snapshot)
}

/// Run repay accounting without transferring the overpayment refund.
///
/// Panics if a positive net repay would burn zero scaled shares.
pub(crate) fn accounting(env: &Env, action: &PoolAction) -> RepayOutcome {
    let (mut cache, position) = ops::load_leg(env, action);
    let amount = action.amount;

    let (burned, overpayment) = cache.resolve_repay(amount, position);
    let net_repay = amount
        .checked_sub(overpayment)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    assert_with_error!(
        env,
        net_repay == 0 || burned.raw() > 0,
        GenericError::RepayRoundsToZeroShares
    );

    let position = position.checked_sub(env, burned);
    cache.burn_debt(burned);

    cache.credit_cash(net_repay);

    let snapshot = cache.commit();
    let mutation = cache.position_mutation(position, net_repay);
    RepayOutcome {
        cache,
        mutation,
        snapshot,
        overpayment,
    }
}
