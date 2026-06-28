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
            s.hub_asset.asset.clone(),
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

#[contractevent(topics = ["strategy", "fee"])]
#[derive(Clone, Debug)]
pub struct StrategyFeeEvent {
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub amount_sent: i128,
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

/// Emits a single market-state snapshot as a one-element batch.
pub(crate) fn publish_market_state(env: &Env, snapshot: MarketStateSnapshot) {
    let mut snapshots = Vec::new(env);
    snapshots.push_back(snapshot);
    publish_market_state_batch(env, snapshots);
}

pub(crate) fn publish_market_params_batch(env: &Env, updates: Vec<PoolMarketParamsEvent>) {
    if updates.is_empty() {
        return;
    }

    PoolMarketParamsBatchEvent { updates }.publish(env);
}

/// Emits a single market-params update as a one-element batch.
pub(crate) fn publish_market_params(env: &Env, asset: Address, params: MarketParamsRaw) {
    let mut updates = Vec::new(env);
    updates.push_back(PoolMarketParamsEvent { asset, params });
    publish_market_params_batch(env, updates);
}

pub(crate) fn publish_strategy_fee(
    env: &Env,
    asset: Address,
    amount: i128,
    fee: i128,
    amount_sent: i128,
) {
    if fee == 0 {
        return;
    }

    StrategyFeeEvent {
        asset,
        amount,
        fee,
        amount_sent,
    }
    .publish(env);
}
