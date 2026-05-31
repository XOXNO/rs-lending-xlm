// Reflector (SEP-40) price provider.

pub(crate) mod client; // Canonical home of the SEP-40 client surface + thin wrappers.
mod spot;
mod twap;

use common::errors::OracleError;
use common::types::{OracleAssetRef, OracleReadMode, ReflectorSourceConfig};
use soroban_sdk::{panic_with_error, Env};

use super::super::observation::{build_observation, check_not_future_at, OracleObservation};
use crate::cache::Cache;

// Re-export the client surface so the provider subtree and validation import from one location.
pub(crate) use client::{
    reflector_base_call, reflector_decimals_call, reflector_lastprice_call, reflector_prices_call,
    reflector_resolution_call, ReflectorAsset, ReflectorPriceData,
};

pub(crate) use twap::min_twap_observations;

pub(crate) fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

pub(crate) fn read_reflector_source(
    cache: &mut Cache,
    config: &ReflectorSourceConfig,
    max_stale: u64,
    required: bool,
) -> Option<OracleObservation> {
    match config.read_mode {
        OracleReadMode::Spot => spot::read_spot(cache.env(), config, required),
        OracleReadMode::Twap(records) => {
            twap::read_twap(cache, config, records, max_stale, required)
        }
    }
}

pub(crate) fn observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    build_observation(env, pd.price, decimals, pd.timestamp, None)
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
}
