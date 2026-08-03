use common::errors::GenericError;
use common::types::{HubAssetKey, HubPayment};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

use common::validation::{expect_invariant, require_non_empty_payments};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ZeroLeg {
    Rejected,

    MeansAll,
}

pub(crate) fn aggregate_positive_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
) -> Vec<HubPayment> {
    aggregate_payments(env, payments, ZeroLeg::Rejected)
}

pub(crate) fn aggregate_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
    zero_leg: ZeroLeg,
) -> Vec<HubPayment> {
    require_non_empty_payments(env, payments);
    let mut order: Vec<HubAssetKey> = Vec::new(env);
    let mut totals: Map<HubAssetKey, i128> = Map::new(env);

    for (hub_asset, amount) in payments {
        let previous = totals.get(hub_asset.clone());
        let next = aggregate_payment_amount(env, previous, amount, zero_leg);

        if previous.is_none() {
            order.push_back(hub_asset.clone());
        }
        totals.set(hub_asset, next);
    }

    let mut result = Vec::new(env);
    for hub_asset in order {
        let amount = expect_invariant(env, totals.get(hub_asset.clone()));
        result.push_back((hub_asset, amount));
    }

    result
}

fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_leg: ZeroLeg,
) -> i128 {
    let zero_means_all = zero_leg == ZeroLeg::MeansAll;
    if amount < 0 || (!zero_means_all && amount == 0) {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    if zero_means_all && (amount == 0 || previous == Some(0)) {
        return 0;
    }

    previous
        .unwrap_or(0)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

#[cfg(test)]
#[path = "../../tests/helpers/utils.rs"]
mod tests;
