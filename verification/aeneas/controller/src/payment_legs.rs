//! Re-exported as `crate::payments` (see `lib.rs`). `ZeroLeg`, `aggregate_positive_payments`, `aggregate_payments` and
//! `aggregate_payment_amount`, copied verbatim from
//! `contracts/controller/src/payments.rs`, and a stand-in for the token
//! transfer it re-exports (`common::token::transfer_amount_measured`, opaque,
//! modelled in `FunsExternal.lean`). The rest of that file is not used by the
//! extracted functions.

use crate::errors::GenericError;
use crate::types::{HubAssetKey, HubPayment};
use crate::validation::{expect_invariant, require_non_empty_payments, require_nonneg_amount};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

/// Meaning of a zero amount when aggregating payment legs.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ZeroLeg {
    /// Require strictly positive amounts.
    Rejected,

    /// Withdraw all; mixing it with a positive amount for the same hub asset
    /// is rejected.
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
/// negative amounts, and overflow. Under `MeansAll`, a zero leg sets that
/// hub asset's total to zero (withdraw all); a zero leg mixed with a positive
/// leg for the same hub asset fails with `InvalidPayments`.
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
        (ZeroLeg::MeansAll, 0, None | Some(0)) => 0,
        (ZeroLeg::MeansAll, 0, Some(_)) | (ZeroLeg::MeansAll, _, Some(0)) => {
            panic_with_error!(env, GenericError::InvalidPayments);
        }
        (_, amount, previous) => previous
            .unwrap_or(0)
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    }
}

/// Stand-in for `common::token::transfer_amount_measured`: transfers `amount`
/// of `asset` from `from` to `to` and returns the balance increase measured at
/// `to`. Aborts with `non_positive_error` unless `amount > 0`. Opaque.
pub(crate) fn transfer_amount_measured(
    _env: &Env,
    _asset: &soroban_sdk::Address,
    _from: &soroban_sdk::Address,
    _to: &soroban_sdk::Address,
    _amount: i128,
    _non_positive_error: GenericError,
) -> i128 {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}
