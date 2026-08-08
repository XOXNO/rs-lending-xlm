//! Claim protocol revenue: burn treasury shares and transfer cash to the owner.

use common::errors::GenericError;
use common::types::{HubAssetKey, PoolAmountMutation};

use soroban_sdk::{panic_with_error, Env};

use stellar_access::ownable;

use crate::cache::Cache;
use crate::{events, guards, ops};

/// Intermediate result after burning claimable revenue and debiting cash.
pub(crate) struct RevenueOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolAmountMutation,
}

/// Claim all currently claimable revenue and pay it to the Ownable owner.
///
/// If nothing is claimable, still emits a market state snapshot and returns
/// zero. Otherwise transfers tokens to the owner after accounting.
///
/// # Returns
///
/// Mutation with `actual_amount` equal to asset units paid out.
pub(crate) fn apply(env: &Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
    let outcome = accounting(env, hub_asset);

    if outcome.mutation.actual_amount == 0 {
        events::emit_market_state(env, outcome.cache.snapshot());
        return outcome.mutation;
    }

    let owner = ownable::get_owner(env)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));
    outcome
        .cache
        .transfer_out(&owner, outcome.mutation.actual_amount);

    events::emit_market_state(env, outcome.cache.snapshot());
    outcome.mutation
}

/// Burn claimable revenue shares, enforce solvency/util guards, debit cash.
pub(crate) fn accounting(env: &Env, hub_asset: HubAssetKey) -> RevenueOutcome {
    let mut cache = ops::renewed_market(env, &hub_asset);

    let net_transfer = cache.burn_claimable_revenue();

    guards::require_utilization_below_max(env, &cache);
    guards::require_solvent_withdraw_state(env, &cache);
    cache.debit_cash(net_transfer);

    cache.commit();
    RevenueOutcome {
        cache,
        mutation: PoolAmountMutation {
            actual_amount: net_transfer,
        },
    }
}
