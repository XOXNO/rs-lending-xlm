//! Stand-in for `contracts/pool/src/storage.rs` (persistent storage), with
//! the production signatures. Marked `opaque` for Charon
//! (`[package.metadata.charon]` in `../Cargo.toml`), so only the signatures
//! reach Lean; their meaning is in `lean/PoolAeneas/FunsExternal.lean`:
//! reads are uninterpreted constants (a theorem holds for every stored
//! value), writes and TTL renewals return `()` and change nothing the
//! extracted code reads back. The bodies here are never executed.

use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw, PoolStateRaw, PoolSyncData};
use soroban_sdk::Env;

pub(crate) fn market_exists(_env: &Env, _hub_asset: &HubAssetKey) -> bool {
    unimplemented!("persistent storage")
}

pub(crate) fn read_params(_env: &Env, _hub_asset: &HubAssetKey) -> MarketParamsRaw {
    unimplemented!("persistent storage")
}

pub(crate) fn read_state(_env: &Env, _hub_asset: &HubAssetKey) -> PoolStateRaw {
    unimplemented!("persistent storage")
}

pub(crate) fn load_state(_env: &Env, _hub_asset: &HubAssetKey) -> PoolStateRaw {
    unimplemented!("persistent storage")
}

pub(crate) fn load_sync_data(_env: &Env, _hub_asset: &HubAssetKey) -> PoolSyncData {
    unimplemented!("persistent storage")
}

pub(crate) fn write_params(_env: &Env, _hub_asset: &HubAssetKey, _params: &MarketParamsRaw) {
    unimplemented!("persistent storage")
}

pub(crate) fn write_state(_env: &Env, _hub_asset: &HubAssetKey, _state: &PoolStateRaw) {
    unimplemented!("persistent storage")
}

pub(crate) fn write_rate_model(
    _env: &Env,
    _hub_asset: &HubAssetKey,
    _model: &InterestRateModel,
) -> MarketParamsRaw {
    unimplemented!("persistent storage")
}

pub(crate) fn renew_market(_env: &Env, _hub_asset: &HubAssetKey) {
    unimplemented!("persistent storage")
}
