//! Cross-contract client trait and call helpers for Reflector price oracle
//! contracts, plus TWAP helpers used to derive a price from a Reflector
//! price history.

use crate::errors::OracleError;
use crate::types::OracleAssetRef;
use soroban_sdk::{contractclient, contracttype, panic_with_error, Address, Env, Symbol, Vec};

/// Identifies a priced asset as understood by a Reflector oracle: either a
/// Stellar contract address or a symbol identifying a non-Stellar asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorAsset {
    Stellar(Address),
    Other(Symbol),
}

/// A single price observation from a Reflector oracle: the price value
/// together with its observation timestamp.
#[contracttype]
#[derive(Clone)]
pub struct ReflectorPriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// Client interface for a Reflector price oracle contract.
#[contractclient(name = "ReflectorClient")]
#[allow(dead_code)]
pub trait ReflectorOracle {
    /// Returns the oracle's base asset, against which prices are quoted.
    fn base(env: Env) -> ReflectorAsset;

    /// Returns the number of decimal places oracle prices are scaled to.
    fn decimals(env: Env) -> u32;

    /// Returns the oracle's configured price resolution.
    fn resolution(env: Env) -> u32;

    /// Returns the most recent price data for `asset`, or `None` if no price is available.
    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    /// Returns up to `records` of the most recent price data points for `asset`,
    /// or `None` if unavailable.
    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}

/// Returns the most recent price data for `asset` from `oracle` via
/// `try_lastprice`. Returns `None` if the call fails or no price is available.
pub fn reflector_last_price(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
) -> Option<ReflectorPriceData> {
    match ReflectorClient::new(env, oracle).try_lastprice(asset) {
        Ok(Ok(data)) => data,
        _ => None,
    }
}

/// Returns up to `records` price data points for `asset` from `oracle` via
/// `try_prices`. Returns `None` if the call fails or no data is available.
pub fn reflector_prices(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
    records: u32,
) -> Option<Vec<ReflectorPriceData>> {
    match ReflectorClient::new(env, oracle).try_prices(asset, &records) {
        Ok(Ok(data)) => data,
        _ => None,
    }
}

/// Returns `oracle`'s resolution via `try_resolution`, or `None` if the call fails.
pub fn try_reflector_resolution(env: &Env, oracle: &Address) -> Option<u32> {
    match ReflectorClient::new(env, oracle).try_resolution() {
        Ok(Ok(resolution)) => Some(resolution),
        _ => None,
    }
}

/// Converts `asset` to a `ReflectorAsset`, mapping a Stellar address or
/// symbol directly. Panics with `OracleError::InvalidOracleTokenType` if
/// `asset` is a `String` reference.
pub fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

/// Computes the arithmetic mean price over `history`. Returns `None` if any
/// price is not positive, the running sum overflows i128, or `history` is empty.
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
