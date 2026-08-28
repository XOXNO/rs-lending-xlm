//! Pool events. Mutation paths emit state snapshots so the hub and indexers
//! track cash, indexes and share totals without re-simulating accrual.

use common::types::{MarketParamsRaw, MarketStateSnapshot};

use soroban_sdk::{contractevent, contracttype, vec, Address, Env, Vec};

/// Positional: hub_id, asset, timestamp, supply_index, borrow_index, cash,
/// supplied, borrowed, revenue.
///
/// `revenue` is OUTSTANDING unclaimed revenue, decremented by every
/// `claim_revenue` — not a cumulative counter. See `ClaimRevenueEvent`.
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

/// Batch of market state updates after a multi-leg operation.
#[contractevent(topics = ["market", "batch_state_update"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolMarketStateBatchEvent {
    pub updates: Vec<PoolMarketStateEvent>,
}

/// Market params payload used when a market is created or reconfigured.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}

/// Batch of market params updates (create / rate-model replace).
#[contractevent(topics = ["market", "batch_params_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsBatchEvent {
    pub updates: Vec<PoolMarketParamsEvent>,
}

/// Fee charged when opening a strategy position with fee collection enabled.
#[contractevent(topics = ["strategy", "fee"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyFeeEvent {
    pub hub_id: u32,
    pub asset: Address,
    /// Gross strategy principal (asset units).
    pub amount: i128,
    /// Protocol fee withheld (asset units).
    pub fee: i128,
    /// Net amount transferred out after fee.
    pub amount_sent: i128,
}

/// No-op on an empty batch.
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

/// One snapshot as a one-element batch.
pub(crate) fn emit_market_state(env: &Env, snapshot: MarketStateSnapshot) {
    emit_market_state_batch(env, vec![env, snapshot]);
}

/// After create or rate-model update.
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

/// Only when `fee` is non-zero.
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
