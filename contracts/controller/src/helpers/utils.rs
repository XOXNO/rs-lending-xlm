//! Small shared utilities: payment aggregation and event context helpers.
//!
//! Pure helpers with no policy or storage side effects.

use common::errors::GenericError;
use controller_interface::types::Payment;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Vec};

use crate::external::sac::sac_transfer_call;
use crate::validation;

/// Deduplicates by asset and sums amounts; panics on zero or negative entries.
pub fn aggregate_positive_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
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
    payments: &Vec<Payment>,
    zero_is_withdraw_all: bool,
) -> Vec<Payment> {
    let mut order: Vec<Address> = Vec::new(env);
    let mut totals: Map<Address, i128> = Map::new(env);

    for (asset, amount) in payments {
        let next =
            aggregate_payment_amount(env, totals.get(asset.clone()), amount, zero_is_withdraw_all);

        if !totals.contains_key(asset.clone()) {
            order.push_back(asset.clone());
        }
        totals.set(asset, next);
    }

    let mut result = Vec::new(env);
    for asset in order {
        let amount = validation::expect_invariant(env, totals.get(asset.clone()));
        result.push_back((asset, amount));
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
    pub action: crate::events::PositionAction,
}
