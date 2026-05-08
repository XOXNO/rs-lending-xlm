#![allow(dead_code)]

use soroban_sdk::{contractclient, contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// SEP-40 asset identifier
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorAsset {
    Stellar(Address),
    Other(Symbol),
}

// ---------------------------------------------------------------------------
// SEP-40 price data
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct ReflectorPriceData {
    pub price: i128,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// SEP-40 oracle client trait
// ---------------------------------------------------------------------------

#[contractclient(name = "ReflectorClient")]
pub trait ReflectorOracle {
    fn base(env: Env) -> ReflectorAsset;

    fn decimals(env: Env) -> u32;

    fn resolution(env: Env) -> u32;

    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}

crate::summarized!(
    reflector::base_summary,
    pub(crate) fn reflector_base_call(env: &Env, oracle: &Address) -> ReflectorAsset {
        ReflectorClient::new(env, oracle).base()
    }
);

crate::summarized!(
    reflector::lastprice_summary,
    pub(crate) fn reflector_lastprice_call(
        env: &Env,
        oracle: &Address,
        asset: &ReflectorAsset,
    ) -> Option<ReflectorPriceData> {
        ReflectorClient::new(env, oracle).lastprice(asset)
    }
);

crate::summarized!(
    reflector::prices_summary,
    pub(crate) fn reflector_prices_call(
        env: &Env,
        oracle: &Address,
        asset: &ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        ReflectorClient::new(env, oracle).prices(asset, &records)
    }
);
