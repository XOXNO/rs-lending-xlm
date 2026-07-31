use common::types::{MarketParamsRaw, MarketStateSnapshot};

use soroban_sdk::{contractevent, contracttype, vec, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolMarketStateEvent(
    pub u32,
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
            s.hub_asset.hub_id,
            s.hub_asset.asset.clone(),
            s.timestamp,
            s.supply_index,
            s.borrow_index,
            s.cash,
            s.supplied,
            s.borrowed,
            s.revenue,
        )
    }
}

#[contractevent(topics = ["market", "batch_state_update"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolMarketStateBatchEvent {
    pub updates: Vec<PoolMarketStateEvent>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}

#[contractevent(topics = ["market", "batch_params_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsBatchEvent {
    pub updates: Vec<PoolMarketParamsEvent>,
}

#[contractevent(topics = ["strategy", "fee"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyFeeEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub amount_sent: i128,
}

pub(crate) fn emit_market_state_batch(env: &Env, snapshots: Vec<MarketStateSnapshot>) {
    if snapshots.is_empty() {
        return;
    }

    let mut updates = Vec::new(env);
    for snapshot in snapshots.iter() {
        updates.push_back(PoolMarketStateEvent::from(&snapshot));
    }
    PoolMarketStateBatchEvent { updates }.publish(env);
}

pub(crate) fn emit_market_state(env: &Env, snapshot: MarketStateSnapshot) {
    emit_market_state_batch(env, vec![env, snapshot]);
}

pub(crate) fn emit_market_params(env: &Env, hub_id: u32, asset: Address, params: MarketParamsRaw) {
    let updates = vec![
        env,
        PoolMarketParamsEvent {
            hub_id,
            asset,
            params,
        },
    ];
    PoolMarketParamsBatchEvent { updates }.publish(env);
}

pub(crate) fn emit_strategy_fee(
    env: &Env,
    hub_id: u32,
    asset: Address,
    amount: i128,
    fee: i128,
    amount_sent: i128,
) {
    if fee == 0 {
        return;
    }

    StrategyFeeEvent {
        hub_id,
        asset,
        amount,
        fee,
        amount_sent,
    }
    .publish(env);
}
