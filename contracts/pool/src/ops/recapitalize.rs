//! Recapitalization: inject cash to cover a market backing shortfall.
//!
//! Only the shortfall amount is applied; any excess is refunded to the payer.
//! The hub is expected to have transferred `amount` into the pool beforehand.

use common::errors::GenericError;
use common::types::{HubAssetKey, PoolAmountMutation};
use common::validation::require_nonneg_amount;

use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::{events, guards, ops};

/// Intermediate result of recap accounting before refund transfer.
pub(crate) struct RecapitalizationOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolAmountMutation,
    /// Unused portion of `amount` returned to the payer.
    pub(crate) refund: i128,
}

/// Apply up to the backing shortfall, refund excess, emit market state.
///
/// # Returns
///
/// Mutation with `actual_amount` equal to cash credited (not including refund).
pub(crate) fn apply(
    env: &Env,
    hub_asset: HubAssetKey,
    payer: Address,
    amount: i128,
) -> PoolAmountMutation {
    let outcome = accounting(env, hub_asset, amount);

    outcome.cache.transfer_out(&payer, outcome.refund);

    events::emit_market_state(env, outcome.cache.snapshot());
    outcome.mutation
}

/// Size and book the cash injection without transferring tokens.
///
/// Credits `min(amount, backing_shortfall)` to cash and commits. `refund` is
/// `amount - applied`.
pub(crate) fn accounting(
    env: &Env,
    hub_asset: HubAssetKey,
    amount: i128,
) -> RecapitalizationOutcome {
    require_nonneg_amount(env, amount);
    let mut cache = ops::renewed_market(env, &hub_asset);

    let applied = amount.min(guards::backing_shortfall(&cache));
    let refund = amount
        .checked_sub(applied)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    cache.credit_cash(applied);
    cache.commit();

    RecapitalizationOutcome {
        cache,
        mutation: PoolAmountMutation {
            actual_amount: applied,
        },
        refund,
    }
}
