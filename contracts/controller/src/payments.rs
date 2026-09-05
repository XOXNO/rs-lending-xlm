use common::errors::GenericError;
use common::types::{HubAssetKey, HubPayment};
use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

pub(crate) use common::token::transfer_amount_measured;
use common::validation::{expect_invariant, require_non_empty_payments, require_nonneg_amount};

/// Returns the measured balance change since `before`, negative for an outflow.
/// Custody accounting uses this delta rather than reported transfer amounts.
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

/// Snapshots `holder`'s balance once per distinct asset address.
pub(crate) fn snapshot_balances(
    env: &Env,
    holder: &Address,
    assets: impl IntoIterator<Item = Address>,
) -> Map<Address, i128> {
    let mut snapshot = Map::new(env);
    for asset in assets {
        if snapshot.contains_key(asset.clone()) {
            continue;
        }
        let balance = token::Client::new(env, &asset).balance(holder);
        snapshot.set(asset, balance);
    }
    snapshot
}

/// Refunds only the controller balance increase since `balance_before`,
/// preserving the pre-existing balance; no-op for a nonpositive delta.
pub(crate) fn refund_controller_balance_delta(
    env: &Env,
    asset: &Address,
    balance_before: i128,
    refund_to: &Address,
) {
    let controller = env.current_contract_address();
    let excess = balance_delta_since(env, asset, &controller, balance_before);
    if excess > 0 {
        token::Client::new(env, asset).transfer(&controller, refund_to, &excess);
    }
}

/// Meaning of a zero amount when aggregating payment legs.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ZeroLeg {
    /// Require strictly positive amounts.
    Rejected,

    /// Withdraw all; overrides positive amounts for the same hub asset.
    MeansAll,
}

/// Aggregates hub-asset payments, requiring every amount to be positive.
pub(crate) fn aggregate_positive_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
) -> Vec<HubPayment> {
    aggregate_payments(env, payments, ZeroLeg::Rejected)
}

/// Aggregates hub-asset payments in first-seen order. Rejects empty input,
/// negative amounts, and overflow. Under `MeansAll`, any zero makes that
/// hub asset's total a persistent withdraw-all sentinel.
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

/// Adds one amount to a hub asset's total under the selected zero policy.
fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_leg: ZeroLeg,
) -> i128 {
    // Validate before the zero sentinel can mask a negative amount.
    require_nonneg_amount(env, amount);

    match (zero_leg, amount, previous) {
        (ZeroLeg::Rejected, 0, _) => {
            panic_with_error!(env, GenericError::AmountMustBePositive);
        }
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
