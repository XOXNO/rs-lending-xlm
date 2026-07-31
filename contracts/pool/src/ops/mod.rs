pub(crate) mod borrow;
pub(crate) mod flash;
pub(crate) mod market;
pub(crate) mod net_settle;
pub(crate) mod recapitalize;
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

pub(crate) fn synced_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    let mut cache = Cache::load(env, hub_asset);
    interest::global_sync(env, &mut cache);
    cache
}

pub(crate) fn renewed_market(env: &Env, hub_asset: &HubAssetKey) -> Cache {
    storage::renew_instance(env);
    synced_market(env, hub_asset)
}

pub(crate) fn load_leg(env: &Env, action: &PoolAction) -> (Cache, Ray) {
    require_nonneg_amount(env, action.amount);
    let cache = synced_market(env, &action.hub_asset);
    (cache, Ray::from(action.position.scaled_amount))
}

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
