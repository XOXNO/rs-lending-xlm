//! Small shared utilities: payment aggregation and event context helpers.
//!
//! Pure helpers with no policy or storage side effects.

use common::errors::GenericError;
use common::types::HubAssetKey;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

use crate::events;
use crate::external::sac::sac_transfer_call;
use crate::positions::HubPayment;
use crate::risk::validation;

/// Deduplicates by hub asset and sums amounts; panics on zero or negative entries.
pub fn aggregate_positive_payments(env: &Env, payments: &Vec<HubPayment>) -> Vec<HubPayment> {
    aggregate_payments(env, payments, false)
}

/// Appends `addr` to `out` if absent (order-preserving dedup).
pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(addr.clone()) {
        out.push_back(addr);
    }
}

/// Transfers a listed SAC amount and returns it.
pub fn transfer_amount(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    non_positive_error: GenericError,
) -> i128 {
    assert_with_error!(env, amount > 0, non_positive_error);
    sac_transfer_call(env, asset, from, to, &amount);
    amount
}

pub fn aggregate_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
    zero_is_withdraw_all: bool,
) -> Vec<HubPayment> {
    validation::require_non_empty_payments(env, payments);
    if payments.len() == 1 {
        // Single-payment fast path: skip the dedup machinery but still enforce
        // the positive-amount gate (and withdraw-all sentinel) the loop applies.
        let (hub_asset, amount) = payments.get_unchecked(0);
        let amount = aggregate_payment_amount(env, None, amount, zero_is_withdraw_all);
        let mut result = Vec::new(env);
        result.push_back((hub_asset, amount));
        return result;
    }
    let mut order: Vec<HubAssetKey> = Vec::new(env);
    let mut totals: Map<HubAssetKey, i128> = Map::new(env);

    for (hub_asset, amount) in payments {
        let next = aggregate_payment_amount(
            env,
            totals.get(hub_asset.clone()),
            amount,
            zero_is_withdraw_all,
        );

        if !totals.contains_key(hub_asset.clone()) {
            order.push_back(hub_asset.clone());
        }
        totals.set(hub_asset, next);
    }

    let mut result = Vec::new(env);
    for hub_asset in order {
        let amount = validation::expect_invariant(env, totals.get(hub_asset.clone()));
        result.push_back((hub_asset, amount));
    }

    result
}

fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_is_withdraw_all: bool,
) -> i128 {
    if amount < 0 || (!zero_is_withdraw_all && amount == 0) {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    if zero_is_withdraw_all && (amount == 0 || previous == Some(0)) {
        return 0;
    }

    previous
        .unwrap_or(0)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

/// Shared context for position and debt update events.
pub(crate) struct EventContext {
    pub caller: Address,
    pub action: events::PositionAction,
}

#[cfg(test)]
#[path = "../../tests/helpers/utils.rs"]
mod tests;
