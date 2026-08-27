#![no_std]

//! Soroban contract entry point for the price aggregator: exposes price and
//! quote queries plus owner-gated oracle administration, and wires the
//! sub-modules that implement resolution, validation, and storage.

mod admin;
mod engine;
mod observation;
mod properties;
mod providers;
mod registry;
mod session;
mod tolerance;
mod validation;

/// Certora formal-verification harness for this contract, sourced from the
/// external `certora/price-aggregator/spec` path.
#[cfg(feature = "certora")]
#[path = "../../../certora/price-aggregator/spec/mod.rs"]
pub mod spec;

#[cfg(test)]
#[path = "../tests/oracle/support.rs"]
mod test_support;

use price_aggregator_interface::PriceAggregatorInterface;
use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, Map, Vec};
use stellar_access::ownable;
use stellar_macros::only_owner;

use common::ttl::renew_instance;
use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};

pub use common::errors::OracleError as Error;

/// Renews the contract instance and returns a new `Session` pre-warmed with
/// `keys`, so repeated lookups of the same keys within a call reuse cached
/// state.
fn warmed_session(env: &Env, keys: &Vec<PriceKey>) -> session::Session {
    renew_instance(env);
    let mut sess = session::Session::new(env);
    sess.warm(keys);
    sess
}

/// The price-aggregator Soroban contract.
#[contract]
pub struct PriceAggregator;

#[contractimpl]
impl PriceAggregator {
    /// Sets `owner` as the contract's owner, emits the ownership-transfer
    /// event, and renews the contract instance.
    pub fn __constructor(env: Env, owner: Address) {
        ownable::set_owner(&env, &owner);
        // `set_owner` writes storage without emitting, so the oracle
        // authority's owner would otherwise be invisible to indexers.
        ownable::emit_ownership_transfer_completed(&env, &owner);
        renew_instance(&env);
    }
}

#[contractimpl]
impl PriceAggregatorInterface for PriceAggregator {
    /// Returns the contract's current owner, if set.
    fn get_owner(env: Env) -> Option<Address> {
        renew_instance(&env);
        ownable::get_owner(&env)
    }

    /// Resolves and returns the price feed for each of `keys`, panicking if
    /// any key fails to resolve to a usable price.
    fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        let mut session = warmed_session(&env, &keys);
        for key in keys.iter() {
            out.set(key.clone(), engine::resolve(&mut session, &key, 0));
        }
        out
    }

    /// Resolves and returns the `PriceStatus` for each of `keys`, without
    /// panicking on individually unusable prices.
    fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let mut out = Map::new(&env);
        let mut session = warmed_session(&env, &keys);
        for key in keys.iter() {
            out.set(key.clone(), engine::resolve_status(&mut session, &key, 0));
        }
        out
    }

    /// Resolves `key` and returns the min and max of its two leg prices as
    /// `(low, high)`, panicking if resolution fails.
    fn price_spread(env: Env, key: PriceKey) -> (i128, i128) {
        let keys = Vec::from_array(&env, [key.clone()]);
        let (_, outcome) = engine::resolve_detailed(&mut warmed_session(&env, &keys), &key, 0);
        (
            outcome.first_wad.min(outcome.second_wad),
            outcome.first_wad.max(outcome.second_wad),
        )
    }

    /// Returns the oracle configuration registered for `key`, if any.
    fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle> {
        renew_instance(&env);
        registry::get_oracle(&env, &key)
    }

    /// Owner-only. Registers or replaces the oracle configuration for `key`.
    #[only_owner]
    fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        admin::set_oracle(&env, key, oracle);
    }

    /// Owner-only. Updates the sanity price bounds for `key`'s oracle.
    #[only_owner]
    fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128) {
        renew_instance(&env);
        admin::set_sanity_band(&env, key, min_wad, max_wad);
    }

    /// Owner-only. Updates the tolerance for `key`'s oracle.
    #[only_owner]
    fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance) {
        renew_instance(&env);
        admin::set_tolerance(&env, key, tolerance);
    }

    /// Upgrades the contract WASM to `new_wasm_hash`, extending instance TTL
    /// first. Restricted to the owner, which in a deployed protocol is the
    /// governance contract, so an upgrade only lands through the timelock.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_instance(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }
}

/// Test-only entry points, compiled for `test` builds and the `testing`
/// feature, that bypass owner authorization and validation for direct registry
/// setup and teardown.
#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    /// Stores `oracle` under `key` directly, without validation or
    /// attestation.
    pub fn seed_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        registry::store_oracle(&env, &key, &oracle);
    }

    /// Removes the oracle configuration registered for `key`.
    pub fn remove_oracle(env: Env, key: PriceKey) {
        renew_instance(&env);
        registry::remove_oracle(&env, &key);
    }
}
