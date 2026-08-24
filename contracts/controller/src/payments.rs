use common::errors::GenericError;
use common::types::{HubAssetKey, HubPayment};
use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

pub(crate) use common::token::transfer_amount_measured;
use common::validation::{expect_invariant, require_non_empty_payments, require_nonneg_amount};

/// Returns `holder`'s current `asset` balance minus `before`: the amount that
/// arrived since that snapshot was taken. Negative when the balance fell.
///
/// Every controller leg that settles through the contract's own custody
/// measures what actually moved rather than trusting a reported figure, so
/// the subtraction lives here once instead of at each call site.
pub(crate) fn balance_delta_since(
    env: &Env,
    asset: &Address,
    holder: &Address,
    before: i128,
) -> i128 {
    token::Client::new(env, asset)
        .balance(holder)
        .checked_sub(before)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ZeroLeg {
    Rejected,

    MeansAll,
}

/// Sums `payments` into per-asset totals, rejecting any zero-amount leg.
pub(crate) fn aggregate_positive_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
) -> Vec<HubPayment> {
    aggregate_payments(env, payments, ZeroLeg::Rejected)
}

/// Sums `payments` into per-asset totals, preserving the order in which
/// assets first appear. Panics if `payments` is empty, if any amount is
/// negative, or if a total would overflow. A zero amount is rejected under
/// `ZeroLeg::Rejected`; under `ZeroLeg::MeansAll` it zeroes that asset's
/// running total as a withdraw-all sentinel, and further amounts for that
/// asset stay at zero.
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

/// Folds `amount` into `previous`'s running total for one asset, applying
/// `zero_leg`'s zero-amount rules. Panics if `amount` is negative or the
/// addition would overflow.
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
#[path = "../tests/helpers/utils.rs"]
mod tests;
