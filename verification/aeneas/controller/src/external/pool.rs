//! Stand-in for `pool_seize_positions_call`
//! (`contracts/controller/src/external/pool.rs`): the cross-contract call
//! that reclassifies seized fee shares as pool revenue. Opaque (Charon
//! `--opaque`); modelled as an uninterpreted effect in `FunsExternal.lean`.

use crate::types::PoolSeizeEntry;
use soroban_sdk::{Address, Env, Vec};

pub(crate) fn pool_seize_positions_call(_env: &Env, _pool_addr: &Address, _entries: &Vec<PoolSeizeEntry>) {
    unimplemented!("opaque: modelled in FunsExternal.lean")
}
