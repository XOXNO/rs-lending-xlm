use common::errors::GenericError;
use common::types::Payment;
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cross_contract::sac::sac_transfer_call;
use crate::validation;

pub use crate::positions::account::{create_account, remove_account};

// Deduplicates and sums payments.
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

// Transfers asset and returns credited amount.
pub fn transfer_and_measure_received(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    balance_decrease_error: GenericError,
) -> i128 {
    if amount <= 0 {
        panic_with_error!(env, balance_decrease_error);
    }
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
