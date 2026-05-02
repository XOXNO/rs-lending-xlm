#![allow(dead_code)]

use soroban_sdk::{contractclient, contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// SEP-40 asset identifier
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
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
    fn decimals(env: Env) -> u32;

    fn resolution(env: Env) -> u32;

    fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData>;

    fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<ReflectorPriceData>>;
}
