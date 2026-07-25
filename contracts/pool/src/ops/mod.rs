//! One module per pool entrypoint. Each module owns the whole story for its
//! operation — accounting, guards, and the token movement that follows — so a
//! reader auditing `withdraw` never has to look anywhere else.
//!
//! Money paths split into an `accounting` half that persists the transition and
//! an `apply` half that adds the SAC transfer. That keeps effects committed
//! before any external call, and lets formal rules verify the accounting half
//! without modelling external token code.

pub(crate) mod borrow;
pub(crate) mod flash;
pub(crate) mod market;
pub(crate) mod net_settle;
pub(crate) mod reconcile;
pub(crate) mod repay;
pub(crate) mod revenue;
pub(crate) mod rewards;
pub(crate) mod seize;
pub(crate) mod strategy;
pub(crate) mod supply;
pub(crate) mod withdraw;

use common::math::fp::Ray;
use common::types::{HubAssetKey, MarketStateSnapshot, PoolAction};
use common::validation::require_nonneg_amount;

use soroban_sdk::{Env, IntoVal, TryFromVal, Val, Vec};

use crate::cache::Cache;
use crate::{events, interest, storage};

/// Loads a market and accrues interest up to the current ledger time.
pub(crate) fn synced_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    let mut cache = Cache::load(env, hub_asset);
    interest::global_sync(env, &mut cache);
    cache
}

/// [`synced_market`] plus the instance-entry renewal, for entrypoints that do
/// not go through [`run_batch`].
pub(crate) fn renewed_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    storage::renew_instance(env);
    synced_market(env, hub_asset)
}

/// Loads the accrued market for `action` together with the caller's current
/// scaled position, rejecting negative amounts up front.
pub(crate) fn load_leg(env: &Env, action: &PoolAction) -> (Cache, Ray) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market(env, &action.hub_asset);
    (cache, Ray::from(action.position.scaled_amount))
}

/// Applies `leg` to every entry in order, then publishes one batched
/// market-state event. Returns the per-entry results in input order.
///
/// Each leg reloads its market, so repeated hub-assets in one batch apply
/// sequentially on top of each other.
pub(crate) fn run_batch<E, R>(
    env: &Env,
    entries: Vec<E>,
    mut leg: impl FnMut(&Env, &E) -> (R, MarketStateSnapshot),
) -> Vec<R>
where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
    R: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    storage::renew_instance(env);

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

/// [`run_batch`] for legs whose only output is the market snapshot.
pub(crate) fn run_batch_without_result<E>(
    env: &Env,
    entries: Vec<E>,
    mut leg: impl FnMut(&Env, &E) -> MarketStateSnapshot,
) where
    E: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone,
{
    storage::renew_instance(env);

    let mut snapshots = Vec::new(env);
    for entry in entries.iter() {
        snapshots.push_back(leg(env, &entry));
    }

    events::emit_market_state_batch(env, snapshots);
}
