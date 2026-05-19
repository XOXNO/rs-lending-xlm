//! Reflector (SEP-40) price provider. The protocol's primary price
//! source. Reads can be spot (`lastprice`) or median-of-N TWAP
//! (`prices`); the dispatcher below picks one based on the source
//! configuration. Each path returns an [`OracleObservation`] that the
//! cross-source composer in `oracle/compose.rs` then anchors and
//! sanity-checks.
//!
//! Split into:
//!   * [`spot`] — single-point `lastprice` read (also the TWAP fallback
//!     path when policy allows).
//!   * [`twap`] — windowed median read with partial-history tolerance,
//!     staleness gating, and an optional spot fallback.
//!
//! `to_reflector_asset` + `observation_from_price_data` are shared
//! helpers used by both submodules.

mod spot;
mod twap;

use common::errors::OracleError;
use common::types::{OracleAssetRef, OracleProviderKind, OracleReadMode, ReflectorSourceConfig};
use soroban_sdk::{panic_with_error, Env};

use super::super::observation::{
    check_not_future_at, normalize_positive_price, OracleObservation,
};
use super::super::reflector::{ReflectorAsset, ReflectorPriceData};
use crate::cache::ControllerCache;

pub(crate) use twap::min_twap_observations;

// `OracleAssetRef::String` is unsupported on Reflector; `Stellar` and
// `Symbol` map to the SEP-40 asset variants.
pub(crate) fn to_reflector_asset(env: &Env, asset: &OracleAssetRef) -> ReflectorAsset {
    match asset {
        OracleAssetRef::Stellar(address) => ReflectorAsset::Stellar(address.clone()),
        OracleAssetRef::Symbol(symbol) => ReflectorAsset::Other(symbol.clone()),
        OracleAssetRef::String(_) => panic_with_error!(env, OracleError::InvalidOracleTokenType),
    }
}

pub(crate) fn read_reflector_source(
    cache: &mut ControllerCache,
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

// Builds an `OracleObservation` from a single Reflector price datum.
// Used by spot reads directly and by TWAP reads to capture the
// newest-observed datum for fallback when the window is rejected.
pub(crate) fn observation_from_price_data(
    env: &Env,
    pd: &ReflectorPriceData,
    decimals: u32,
    read_mode: OracleReadMode,
) -> OracleObservation {
    check_not_future_at(env, env.ledger().timestamp(), pd.timestamp);
    OracleObservation {
        price_wad: normalize_positive_price(env, pd.price, decimals),
        raw_price: pd.price,
        raw_decimals: decimals,
        observed_at: pd.timestamp,
        published_at: None,
        provider: OracleProviderKind::ReflectorSep40,
        read_mode,
    }
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
