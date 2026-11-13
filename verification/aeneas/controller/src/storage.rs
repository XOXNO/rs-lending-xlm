//! Stand-ins for `iter_typed_positions` and `iter_debt_positions`
//! (`contracts/controller/src/storage/account.rs`). Production returns
//! `map.iter().map(|(key, raw)| (key, AccountPosition::from(&raw)))` as an
//! `impl Iterator`; these build the same items, in the same order, into a
//! vector and return its iterator (Aeneas does not translate `impl Trait`
//! return types or the `map` adapter). The conversion is the production
//! `From` impl (`types.rs`).

use crate::types::{AccountPosition, AccountPositionRaw, DebtPosition, DebtPositionRaw, HubAssetKey};
use soroban_sdk::{Env, Map, Vec, VecIter};

pub(crate) fn iter_typed_positions(
    map: &Map<HubAssetKey, AccountPositionRaw>,
) -> VecIter<(HubAssetKey, AccountPosition)> {
    let mut out = Vec::new(&Env);
    for (key, raw) in map.iter() {
        out.push_back((key, AccountPosition::from(&raw)));
    }
    out.into_iter()
}

pub(crate) fn iter_debt_positions(
    map: &Map<HubAssetKey, DebtPositionRaw>,
) -> VecIter<(HubAssetKey, DebtPosition)> {
    let mut out = Vec::new(&Env);
    for (key, raw) in map.iter() {
        out.push_back((key, DebtPosition::from(&raw)));
    }
    out.into_iter()
}
