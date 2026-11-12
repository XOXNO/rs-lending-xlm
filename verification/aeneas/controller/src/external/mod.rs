//! Stand-in for the pool index fetch the controller `Context` makes on a
//! cache miss (`contracts/controller/src/external/pool.rs`,
//! `fetch_pool_bulk_indexes` with a one-key request). Opaque (Charon
//! `--opaque`); its Lean model is an unspecified function of the key
//! (`FunsExternal.lean`).

use crate::types::{HubAssetKey, MarketIndexRaw};
use soroban_sdk::Env;

pub(crate) fn fetch_market_index(_env: &Env, _hub_asset: &HubAssetKey) -> MarketIndexRaw {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}

pub(crate) mod pool;
