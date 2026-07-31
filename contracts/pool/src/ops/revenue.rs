use common::errors::GenericError;
use common::types::{HubAssetKey, PoolAmountMutation};

use soroban_sdk::{panic_with_error, Env};

use stellar_access::ownable;

use crate::cache::Cache;
use crate::{events, guards, ops};

pub(crate) struct RevenueOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolAmountMutation,
}

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
