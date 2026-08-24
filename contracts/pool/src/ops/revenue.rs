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

/// Claims all currently claimable revenue and pays it to the Ownable owner.
/// Emits a market state snapshot in all cases. If nothing is claimable, returns
/// a mutation with `actual_amount` zero and performs no transfer; otherwise
/// transfers the claimed amount to the owner after accounting.
pub(crate) fn apply(env: &Env, hub_asset: HubAssetKey) -> PoolAmountMutation {
    let outcome = accounting(env, hub_asset);

    if outcome.mutation.actual_amount != 0 {
        let owner = ownable::get_owner(env)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::OwnerNotSet));
        outcome
            .cache
            .transfer_out(&owner, outcome.mutation.actual_amount);
    }

    events::emit_market_state(env, outcome.cache.snapshot());
    outcome.mutation
}

/// Renews and syncs the market, burns claimable revenue shares, enforces the
/// utilization and solvency guards, and debits the net transfer from cash.
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
