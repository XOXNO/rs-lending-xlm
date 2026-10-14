//! Claim protocol revenue: burn revenue shares and transfer cash to the owner.
//!
//! While the market carries debt, a claim draws only the cash above the
//! liquidation buffer (`Cache::claimable_cash`), and pays nothing if that
//! payout would raise utilization past `min(max_utilization, 95%)`
//! (`Cache::claim_utilization_room`): revenue accrued on uncollectible
//! interest must not take the cash that liquidation or cleanup of that debt
//! needs. The claim pays less, or nothing, rather than reverting, so it never
//! fails the controller's claim batch over a market it cannot pay.

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

/// Claims all currently claimable revenue, bounded by the claimable cash, and
/// pays it to the Ownable owner. Emits a market state snapshot in all cases.
/// If nothing is claimable, returns a mutation with `actual_amount` zero and
/// performs no transfer.
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
///
/// A claim that moves nothing burns nothing, so it only accrues, like
/// `update_indexes`, and skips both guards: a market already past its
/// utilization ceiling, or whose supply shares all left while rounding dust
/// was borrowed, must not fail the controller's whole claim batch over a zero
/// payout. A positive payout stays within the claim utilization room, so it
/// neither breaches the ceiling nor burns the last supply share under debt;
/// both guards still run on it as defence in depth.
pub(crate) fn accounting(env: &Env, hub_asset: HubAssetKey) -> RevenueOutcome {
    let mut cache = ops::renewed_market(env, &hub_asset);

    let net_transfer = cache.burn_claimable_revenue();

    if net_transfer != 0 {
        guards::require_utilization_below_max(env, &cache);
        guards::require_supply_for_debt(env, &cache);
    }
    cache.debit_cash(net_transfer);

    cache.commit();
    RevenueOutcome {
        cache,
        mutation: PoolAmountMutation {
            actual_amount: net_transfer,
        },
    }
}
