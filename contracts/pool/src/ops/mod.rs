//! Market mutation operations invoked from the pool's public interface.
//!
//! Each submodule implements one logical action (supply, borrow, …). Shared
//! helpers here load an interest-synced [`Cache`], run multi-leg batches, and
//! emit market state events after each batch.

pub(crate) mod borrow;
pub(crate) mod flash;
pub(crate) mod market;
pub(crate) mod net_settle;
pub(crate) mod recapitalize;
pub(crate) mod repay;
pub(crate) mod revenue;
pub(crate) mod seize;
pub(crate) mod strategy;
pub(crate) mod supply;
pub(crate) mod withdraw;

use common::math::fp::Ray;
use common::ttl::renew_instance;
use common::types::{HubAssetKey, MarketStateSnapshot, PoolAction};
use common::validation::require_nonneg_amount;

use soroban_sdk::{Env, IntoVal, TryFromVal, Val, Vec};

use crate::cache::Cache;
use crate::{events, interest};

/// Loads a market cache and accrues interest through the current ledger time.
pub(crate) fn synced_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    let mut cache = Cache::load(env, hub_asset);
    interest::global_sync(env, &mut cache);
    cache
}

/// Renews instance TTL, then loads and accrues the market.
pub(crate) fn renewed_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    renew_instance(env);
    synced_market(env, hub_asset)
}

/// Validates `action.amount ≥ 0`, syncs the market, and returns (cache, scaled position).
pub(crate) fn load_leg(env: &Env, action: &PoolAction) -> (Cache, Ray) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market(env, &action.hub_asset);
    (cache, Ray::from(action.position.scaled_amount))
}

/// Runs a multi-entry batch: renews the instance, applies `leg` per entry, and emits state events.
///
/// Each leg returns a result `R` plus a [`MarketStateSnapshot`] for the event batch.
pub(crate) fn run_batch<E, R>(
    env: &Env,
    entries: Vec<E>,
    mut leg: impl FnMut(&Env, &E) -> (R, MarketStateSnapshot),
) -> Vec<R>
where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
    R: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    renew_instance(env);

    let mut results = Vec::new(env);
    let mut snapshots = Vec::new(env);
    for entry in entries.iter() {
        let (result, snapshot) = leg(env, &entry);
        results.push_back(result);
        snapshots.push_back(snapshot);
    }

    events::emit_market_state_batch(env, snapshots);
    results
}
