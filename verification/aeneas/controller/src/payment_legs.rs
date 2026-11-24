//! Re-exported as `crate::payments` (see `lib.rs`). `aggregate_positive_payments`
//! and `aggregate_payment_amount`, copied verbatim from
//! `contracts/controller/src/payments.rs`, and a stand-in for the token
//! transfer it re-exports (`common::token::transfer_amount_measured`, opaque,
//! modelled in `FunsExternal.lean`). The rest of that file is not used by the
//! extracted functions.

use crate::errors::GenericError;
use crate::types::{HubAssetKey, HubPayment};
use crate::validation::{expect_invariant, require_non_empty_payments, require_positive_amount};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

/// Aggregates hub-asset payments in first-seen order. Rejects empty input,
/// zero or negative amounts, and overflow.
pub(crate) fn aggregate_positive_payments(
    env: &Env,
    payments: &Vec<HubPayment>,
) -> Vec<HubPayment> {
    require_non_empty_payments(env, payments);
    let mut order: Vec<HubAssetKey> = Vec::new(env);
    let mut totals: Map<HubAssetKey, i128> = Map::new(env);

    for (hub_asset, amount) in payments {
        let previous = totals.get(hub_asset.clone());
        let next = aggregate_payment_amount(env, previous, amount);

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

/// Adds one positive amount to a hub asset's total.
fn aggregate_payment_amount(env: &Env, previous: Option<i128>, amount: i128) -> i128 {
    require_positive_amount(env, amount);
    previous
        .unwrap_or(0)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
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
