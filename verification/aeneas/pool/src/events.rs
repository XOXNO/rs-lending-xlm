//! Stand-in for `contracts/pool/src/events.rs` (event emission), with the
//! production signatures. Marked `opaque` for Charon; in Lean each emitter
//! returns `()` (`FunsExternal.lean`): events write only the host event
//! buffer, which nothing in the extracted code reads.

use common::types::{HubAssetKey, MarketParamsRaw, MarketStateSnapshot};
use soroban_sdk::{Address, Env, Vec};

pub(crate) fn emit_market_state_batch(_env: &Env, _snapshots: Vec<MarketStateSnapshot>) {
    unimplemented!("event emission")
}

pub(crate) fn emit_market_state(_env: &Env, _snapshot: MarketStateSnapshot) {
    unimplemented!("event emission")
}

pub(crate) fn emit_market_params(_env: &Env, _hub_id: u32, _asset: Address, _params: MarketParamsRaw) {
    unimplemented!("event emission")
}

pub(crate) fn emit_strategy_fee(
    _env: &Env,
    _hub_id: u32,
    _asset: Address,
    _amount: i128,
    _fee: i128,
    _amount_sent: i128,
) {
    unimplemented!("event emission")
}

pub(crate) fn emit_market_value_ceiling(_env: &Env, _hub_asset: &HubAssetKey) {
    unimplemented!("event emission")
}
