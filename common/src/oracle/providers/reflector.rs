//! Reflector SEP-40 client and call wrappers.

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
#[allow(dead_code)] // Required: trait exists only for the macro to generate the client proxy.
pub trait ReflectorOracle {
    fn base(env: Env) -> ReflectorAsset;

    fn decimals(env: Env) -> u32;

    fn resolution(env: Env) -> u32;

    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}
// Thin wrappers isolate external Reflector calls.

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

/// Reads provider decimals during market-oracle config validation.
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

// Min observations for trusted TWAP. Floor of 2 rules out single-sample
// "TWAPs"; larger windows accept partial history above ceil(records/2).
pub fn min_twap_observations(records: u32) -> u32 {
    core::cmp::max(2, records.div_ceil(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Covers the `OracleAssetRef::Symbol` mapping in `to_reflector_asset`.
    // The production harness only registers `Stellar(addr)` markets so this
    // variant has no organic exercise — direct unit test fills the gap.
    #[test]
    fn test_to_reflector_asset_symbol_maps_to_other() {
        let env = Env::default();
        let symbol = soroban_sdk::Symbol::new(&env, "USD");
        let asset = OracleAssetRef::Symbol(symbol.clone());
        let result = to_reflector_asset(&env, &asset);
        match result {
            ReflectorAsset::Other(s) => assert_eq!(s, symbol),
            _ => panic!("expected ReflectorAsset::Other"),
        }
    }

    // `OracleAssetRef::String` is unsupported on Reflector — must panic
    // with `InvalidOracleTokenType`.
    #[test]
    #[should_panic]
    fn test_to_reflector_asset_string_panics() {
        let env = Env::default();
        let asset = OracleAssetRef::String(soroban_sdk::String::from_str(&env, "USDC"));
        let _ = to_reflector_asset(&env, &asset);
    }

    #[test]
    fn test_min_twap_observations_clamps_and_rounds_up() {
        assert_eq!(min_twap_observations(0), 2);
        assert_eq!(min_twap_observations(1), 2);
        assert_eq!(min_twap_observations(2), 2);
        assert_eq!(min_twap_observations(3), 2);
        assert_eq!(min_twap_observations(4), 2);
        assert_eq!(min_twap_observations(5), 3);
        assert_eq!(min_twap_observations(12), 6);
    }
}
