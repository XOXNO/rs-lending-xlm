//! Reflector SEP-40 client and call wrappers.

use crate::errors::{GenericError, OracleError};
use crate::types::OracleAssetRef;
use soroban_sdk::{
    assert_with_error, contractclient, contracttype, panic_with_error, Address, Env, Symbol, Vec,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorAsset {
    Stellar(Address),
    Other(Symbol),
}

#[contracttype]
#[derive(Clone)]
pub struct ReflectorPriceData {
    pub price: i128,
    pub timestamp: u64,
}

#[contractclient(name = "ReflectorClient")]
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait ReflectorOracle {
    fn base(env: Env) -> ReflectorAsset;

    fn decimals(env: Env) -> u32;

    fn resolution(env: Env) -> u32;

    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}

pub fn reflector_base_call(env: &Env, oracle: &Address) -> ReflectorAsset {
    ReflectorClient::new(env, oracle).base()
}

pub fn reflector_lastprice_call(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
) -> Option<ReflectorPriceData> {
    ReflectorClient::new(env, oracle).lastprice(asset)
}

pub fn reflector_prices_call(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
    records: u32,
) -> Option<Vec<ReflectorPriceData>> {
    ReflectorClient::new(env, oracle).prices(asset, &records)
}

pub fn reflector_decimals_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).decimals()
}

pub fn reflector_resolution_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).resolution()
}

pub fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

/// Min TWAP samples: ≥2, and ≥ ceil(records/2) for larger windows.
pub fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(2, records.div_ceil(2))
}

/// Arithmetic mean of the history sample prices (raw, pre-normalization) — the
/// single definition of "the TWAP price". Shared by the controller read path
/// and the governance propose-time containment probe so both derive the exact
/// same value; a divergent reimplementation is what would let a config pass
/// propose yet revert `SanityBoundViolated` at read time. Rejects a non-positive
/// sample (`InvalidPrice`) and a sum overflow (`MathOverflow`). Callers must
/// pass a non-empty history (TWAP reads guard on `min_twap_observations`).
pub fn twap_mean_price(env: &Env, history: &Vec<ReflectorPriceData>) -> i128 {
    let mut sum: i128 = 0;
    for pd in history.iter() {
        assert_with_error!(env, pd.price > 0, OracleError::InvalidPrice);
        sum = sum
            .checked_add(pd.price)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }
    sum / (history.len() as i128)
}

/// Non-panicking [`twap_mean_price`]: `None` for a non-positive sample or a
/// sum overflow, for the aggregator's soft status path.
pub fn try_twap_mean_price(history: &Vec<ReflectorPriceData>) -> Option<i128> {
    let mut sum: i128 = 0;
    for pd in history.iter() {
        if pd.price <= 0 {
            return None;
        }
        sum = sum.checked_add(pd.price)?;
    }
    let len = history.len();
    if len == 0 {
        return None;
    }
    Some(sum / (len as i128))
}

#[cfg(test)]
#[path = "../../../tests/oracle/providers/reflector.rs"]
mod tests;
