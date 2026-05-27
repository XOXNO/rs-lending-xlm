//! Small shared utilities: payment aggregation, event context helpers,
//! and re-exports of account lifecycle from helpers.
//!
//! These are called from many flows; they are pure or near-pure and have
//! no policy or storage side effects of their own.

use common::errors::GenericError;
use common::types::Payment;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Map, Symbol, Vec};

use crate::cross_contract::sac::sac_transfer_call;
use crate::validation;

pub(crate) use crate::helpers::{create_account, remove_account};

/// Deduplicates by asset and sums positive amounts only. Zero and negative
/// entries are dropped (used by every mutating entrypoint before plan execution).
pub fn aggregate_positive_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
    aggregate_payments(env, payments, false)
}

// Asset addresses from an already-aggregated payment plan. The aggregation
// step (`aggregate_positive_payments` / `aggregate_payments`) guarantees
// uniqueness, so this is just a tuple-to-address projection — no dedup.
pub fn plan_assets(env: &Env, plan: &Vec<Payment>) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);
    for (asset, _) in plan {
        out.push_back(asset);
    }
    out
}

/// Appends `addr` to `out` only if not already present (order-preserving dedup).
pub fn push_unique_address(out: &mut Vec<Address>, addr: Address) {
    if !out.contains(addr.clone()) {
        out.push_back(addr);
    }
}

/// Performs the SAC transfer and returns the actual credited amount measured
/// at the recipient (defends against fee-on-transfer or hook surprises).
pub fn transfer_and_measure_received(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    balance_decrease_error: GenericError,
) -> i128 {
    assert_with_error!(env, amount > 0, balance_decrease_error);
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

/// Context for emitting position/debt update events from multiple paths
/// (repay, withdraw, liquidation, strategies). Moved out of positions/ so it
/// can be shared without pulling in the whole positions module.
pub(crate) struct EventContext {
    pub caller: Address,
    pub action: Symbol,
}
