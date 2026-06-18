use common::types::{MarketParamsRaw, MarketStateSnapshot};
use soroban_sdk::{contractevent, contracttype, Address, Env, Vec};

/// Pool market accounting snapshot emitted after successful pool mutations.
///
/// Field order is wire ABI; do not reorder:
/// `[asset, timestamp, supply_index_ray, borrow_index_ray, reserves_ray,
///   supplied_ray, borrowed_ray, revenue_ray]`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketStateEvent(
    pub Address,
    pub u64,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
    pub i128,
);

impl From<&MarketStateSnapshot> for PoolMarketStateEvent {
    fn from(s: &MarketStateSnapshot) -> Self {
        Self(
            s.asset.clone(),
            s.timestamp,
            s.supply_index_ray,
            s.borrow_index_ray,
            s.reserves_ray,
            s.supplied_ray,
            s.borrowed_ray,
            s.revenue_ray,
        )
    }
}

#[contractevent(topics = ["market", "batch_state_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct PoolMarketStateBatchEvent {
    pub updates: Vec<PoolMarketStateEvent>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsEvent {
    pub asset: Address,
    pub params: MarketParamsRaw,
}

#[contractevent(topics = ["market", "batch_params_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsBatchEvent {
    pub updates: Vec<PoolMarketParamsEvent>,
}

pub(crate) fn publish_market_state_batch(env: &Env, snapshots: Vec<MarketStateSnapshot>) {
    if snapshots.is_empty() {
        return;
    }

    let mut updates = Vec::new(env);
    for snapshot in snapshots.iter() {
        updates.push_back(PoolMarketStateEvent::from(&snapshot));
    }
    PoolMarketStateBatchEvent { updates }.publish(env);
}

pub(crate) fn publish_market_params_batch(env: &Env, updates: Vec<PoolMarketParamsEvent>) {
    if updates.is_empty() {
        return;
    }

    PoolMarketParamsBatchEvent { updates }.publish(env);
}
