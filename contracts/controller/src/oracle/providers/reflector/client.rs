//! Reflector SEP-40 client interface (canonical home).
//!
//! This module owns the complete external contract surface for Reflector oracles:
//!
//! - `ReflectorAsset` and `ReflectorPriceData` — the on-chain types used in the SEP-40 ABI.
//! - `ReflectorOracle` trait annotated with `#[contractclient(name = "ReflectorClient")]`.
//! - The thin `pub(crate)` wrapper functions that are the only allowed way for this
//!   crate to perform cross-contract calls to a Reflector oracle.
//!
//! # Design Principles
//! - All direct interaction with the external Reflector contract happens here.
//! - Consumption logic (Spot vs TWAP dispatch, asset mapping, fallback behavior,
//!   observation construction) lives in sibling modules (`spot.rs`, `twap.rs`).
//! - This separation makes the security boundary obvious and keeps the Certora
//!   harness surface small and stable.
//!
//! The `#[allow(dead_code)]` on the trait is required and intentional. The trait
//! exists solely for the `contractclient` macro to generate the proxy type
//! `ReflectorClient`. Its methods are never called directly as Rust trait methods.

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
// Thin wrappers — the only allowed call sites into external Reflector oracles.
// These exist so that Certora can replace the cross-contract behavior with
// sound nondeterministic models without touching the rest of the oracle logic.

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

/// Additional wrappers used exclusively during market oracle configuration
/// validation. They are kept here so every cross-contract call to a Reflector
/// oracle is still routed through this module.
pub(crate) fn reflector_decimals_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).decimals()
}

pub(crate) fn reflector_resolution_call(env: &Env, oracle: &Address) -> u32 {
    ReflectorClient::new(env, oracle).resolution()
}
