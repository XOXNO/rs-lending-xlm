//! Pool event definitions and emit helpers. Market state and params
//! updates are emitted as batches (single-element batches for one market) so
//! indexers consume one topic per flow; empty state batches and zero-fee
//! strategy events are suppressed.
//!
//! Events are defined with `#[contractevent]` (or `#[contracttype]` for
//! batch payloads) and always emitted via the `emit_*` helpers in this module.

use common::types::{MarketParamsRaw, MarketStateSnapshot};

use soroban_sdk::{contractevent, contracttype, vec, Address, Env, Vec};

// ################## EVENTS ##################

/// Pool market accounting snapshot. Field order is wire ABI:
/// `[hub_id, asset, timestamp, supply_index, borrow_index, cash,
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

/// Batch of per-market state snapshots emitted after mutating flows.
#[contractevent(topics = ["market", "batch_state_update"], data_format = "single-value")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolMarketStateBatchEvent {
    pub updates: Vec<PoolMarketStateEvent>,
}

/// One market's rate-model parameter update.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub params: MarketParamsRaw,
}

/// Batch of market rate-model parameter updates.
#[contractevent(topics = ["market", "batch_params_update"], data_format = "single-value")]
#[derive(Clone, Debug)]
pub struct PoolMarketParamsBatchEvent {
    pub updates: Vec<PoolMarketParamsEvent>,
}

/// Protocol fee charged on a strategy borrow.
#[contractevent(topics = ["strategy", "fee"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyFeeEvent {
    pub hub_id: u32,
    pub asset: Address,
    pub amount: i128,
    pub fee: i128,
    pub amount_sent: i128,
}

/// Emits a batch of market-state snapshots; an empty batch is suppressed.
///
/// # Events
///
/// * topics - `["market", "batch_state_update"]`
/// * data - `[updates: Vec<PoolMarketStateEvent>]`
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

/// Emits a single market-state snapshot as a one-element batch.
///
/// # Events
///
/// * topics - `["market", "batch_state_update"]`
/// * data - `[updates: Vec<PoolMarketStateEvent>]`
pub(crate) fn emit_market_state(env: &Env, snapshot: MarketStateSnapshot) {
    emit_market_state_batch(env, vec![env, snapshot]);
}

/// Emits a single market-params update as a one-element batch.
///
/// # Events
///
/// * topics - `["market", "batch_params_update"]`
/// * data - `[updates: Vec<PoolMarketParamsEvent>]`
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

/// Emits a strategy-fee event; zero-fee strategy borrows are suppressed.
///
/// # Events
///
/// * topics - `["strategy", "fee"]`
/// * data - `[hub_id: u32, asset: Address, amount: i128, fee: i128, amount_sent: i128]`
pub(crate) fn emit_strategy_fee(
    env: &Env,
    hub_id: u32,
    asset: Address,
    amount: i128,
    fee: i128,
    amount_sent: i128,
) {
    // Zero-fee strategy borrows have nothing to report.
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
