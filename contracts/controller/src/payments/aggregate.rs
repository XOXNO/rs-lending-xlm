//! Nets a list of [`HubPayment`] entries down to one total per hub asset,
//! preserving the order in which each asset first appears.

use common::errors::GenericError;
use common::types::{HubAssetKey, HubPayment};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

use common::validation::{expect_invariant, require_non_empty_payments, require_nonneg_amount};

/// Selects how a zero payment amount is handled while aggregating per-asset totals.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ZeroLeg {
    /// A zero amount panics with `GenericError::AmountMustBePositive`.
    Rejected,

    /// A zero amount is a "withdraw all" sentinel: it forces the running total for
    /// that asset to zero, and that zero stays sticky against later amounts for the
    /// same asset within the same aggregation pass.
    MeansAll,
}

/// Aggregates `payments` into per-asset totals, rejecting any zero-amount entry.
pub(crate) fn aggregate_positive_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
) -> Vec<HubPayment> {
    aggregate_payments(env, payments, ZeroLeg::Rejected)
}

/// Sums `payments` by hub asset, in the order each asset first appears, applying
/// `zero_leg`'s policy to zero amounts. Panics if `payments` is empty, if any amount
/// is negative, if `zero_leg` is `Rejected` and an amount is zero, or if summing
/// overflows `i128`.
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

/// Combines `amount` with the `previous` running total for one hub asset under
/// `zero_leg`'s zero-handling policy. Panics if `amount` is negative, if `zero_leg`
/// is `Rejected` and `amount` is zero, or if the addition overflows `i128`.
fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_leg: ZeroLeg,
) -> i128 {
    // Negative is always fatal and must run before sticky-zero arms.
    // Otherwise MeansAll + previous==Some(0) would swallow negatives as 0.
    require_nonneg_amount(env, amount);

    match (zero_leg, amount, previous) {
        (ZeroLeg::Rejected, 0, _) => {
            panic_with_error!(env, GenericError::AmountMustBePositive);
        }
        // Withdraw-all sentinel, and sticky zero once a MeansAll total is 0.
        (ZeroLeg::MeansAll, 0, _) | (ZeroLeg::MeansAll, _, Some(0)) => 0,
        (_, amount, previous) => previous
            .unwrap_or(0)
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    }
}

#[cfg(test)]
#[path = "../../tests/helpers/utils.rs"]
mod tests;
