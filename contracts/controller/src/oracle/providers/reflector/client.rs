//! Reflector SEP-40 client and call wrappers.

use soroban_sdk::{contractclient, contracttype, Address, Env, Symbol, Vec};

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

pub(crate) fn reflector_base_call(env: &Env, oracle: &Address) -> ReflectorAsset {
    ReflectorClient::new(env, oracle).base()
}

pub(crate) fn reflector_lastprice_call(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
) -> Option<ReflectorPriceData> {
    ReflectorClient::new(env, oracle).lastprice(asset)
}

pub(crate) fn reflector_prices_call(
    env: &Env,
    oracle: &Address,
    asset: &ReflectorAsset,
    records: u32,
) -> Option<Vec<ReflectorPriceData>> {
    ReflectorClient::new(env, oracle).prices(asset, &records)
}

/// Reads provider decimals during market-oracle config validation.
pub(crate) fn reflector_decimals_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).decimals()
}

pub(crate) fn reflector_resolution_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).resolution()
}
