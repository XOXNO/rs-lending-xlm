//! Market-state, params, and strategy-fee events.
//!
//! | Emit | When |
//! |---|---|
//! | [`emit_market_state_batch`] / [`emit_market_state`] | After money-path commits |
//! | [`emit_market_params`] | After create / replace rate model |
//! | [`emit_strategy_fee`] | After strategy borrow when fee > 0 |
//!
//! Empty batches and zero-fee strategy events are suppressed.

use common::types::{MarketParamsRaw, MarketStateSnapshot};

use soroban_sdk::{contractevent, contracttype, vec, Address, Env, Vec};

/// Pool market accounting snapshot.
///
/// Positional, not named: field order *is* the ABI. Reordering two of the six
/// `i128` fields compiles silently and breaks every indexer, so build it only
/// through the [`From`] impl below, never from a bare tuple literal.
///
/// Order: `[hub_id, asset, timestamp, supply_index, borrow_index, cash,
///   supplied, borrowed, revenue]`.
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

/// Strategy borrow fee. `amount` is the gross borrow, `fee` the withheld
/// flash-loan fee, and `amount_sent` the net paid out (`amount - fee`).
#[contractevent(topics = ["strategy", "fee"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyFeeEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub amount_sent: i128,
}

/// Publishes one batched market-state event. An empty batch emits nothing.
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

/// Publishes a one-entry market-state batch.
pub(crate) fn emit_market_state(env: &Env, snapshot: MarketStateSnapshot) {
    emit_market_state_batch(env, vec![env, snapshot]);
}

/// Publishes a one-entry params batch.
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

/// Publishes a strategy fee event. A zero fee emits nothing.
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
