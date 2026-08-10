use crate::errors::OracleError;
use crate::types::OracleAssetRef;
use soroban_sdk::{contractclient, contracttype, panic_with_error, Address, Env, Symbol, Vec};

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
#[allow(dead_code)]
pub trait ReflectorOracle {
    fn base(env: Env) -> ReflectorAsset;

    fn decimals(env: Env) -> u32;

    fn resolution(env: Env) -> u32;

    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}

pub fn reflector_base(env: &Env, oracle: &Address) -> ReflectorAsset {
    ReflectorClient::new(env, oracle).base()
}

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

pub fn reflector_decimals(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).decimals()
}

pub fn reflector_resolution(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).resolution()
}

pub fn try_reflector_resolution(env: &Env, oracle: &Address) -> Option<u32> {
    match ReflectorClient::new(env, oracle).try_resolution() {
        Ok(Ok(resolution)) => Some(resolution),
        _ => None,
    }
}

pub fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

pub fn min_twap_observations(records: u32) -> u32 {
    records
}

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
