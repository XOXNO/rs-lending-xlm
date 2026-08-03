use common::errors::GenericError;
use common::types::{HubAssetKey, PoolAmountMutation};
use common::validation::require_nonneg_amount;

use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::{events, guards, ops};

pub(crate) struct RecapitalizationOutcome {
    pub(crate) cache: Cache,
    pub(crate) mutation: PoolAmountMutation,
    pub(crate) refund: i128,
}

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
