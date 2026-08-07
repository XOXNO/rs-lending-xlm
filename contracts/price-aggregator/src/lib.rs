#![no_std]

mod admin;
mod engine;
mod observation;
mod properties;
mod providers;
mod registry;
mod session;
mod tolerance;
mod validation;

#[cfg(feature = "certora")]
#[path = "../../../certora/price-aggregator/spec/mod.rs"]
pub mod spec;

#[cfg(test)]
#[path = "../tests/oracle/support.rs"]
mod test_support;

use soroban_sdk::{contract, contractimpl, Address, Env, Map, Vec};
use stellar_access::ownable;
use stellar_macros::only_owner;

use common::constants::{TTL_BUMP_INSTANCE, TTL_THRESHOLD_INSTANCE};
use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};

pub use common::errors::OracleError as Error;

fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

fn warmed_session(env: &Env, keys: &Vec<PriceKey>) -> session::Session {
    renew_instance(env);
    let mut sess = session::Session::new(env);
    sess.warm(keys);
    sess
}

#[contract]
pub struct PriceAggregator;

#[contractimpl]
impl PriceAggregator {
    pub fn __constructor(env: Env, owner: Address) {
        ownable::set_owner(&env, &owner);
        renew_instance(&env);
    }

    pub fn get_owner(env: Env) -> Option<Address> {
        renew_instance(&env);
        ownable::get_owner(&env)
    }

    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        let mut session = warmed_session(&env, &keys);
        for key in keys.iter() {
            out.set(key.clone(), engine::resolve(&mut session, &key, 0));
        }
        out
    }

    pub fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let mut out = Map::new(&env);
        let mut session = warmed_session(&env, &keys);
        for key in keys.iter() {
            out.set(key.clone(), engine::resolve_status(&mut session, &key, 0));
        }
        out
    }

    pub fn price_spread(env: Env, key: PriceKey) -> (i128, i128) {
        let keys = Vec::from_array(&env, [key.clone()]);
        let (_, outcome) = engine::resolve_detailed(&mut warmed_session(&env, &keys), &key, 0);
        (
            outcome.first_wad.min(outcome.second_wad),
            outcome.first_wad.max(outcome.second_wad),
        )
    }

    pub fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle> {
        renew_instance(&env);
        registry::get_oracle(&env, &key)
    }

    #[only_owner]
    pub fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        admin::set_oracle(&env, key, oracle);
    }

    #[only_owner]
    pub fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128) {
        renew_instance(&env);
        admin::set_sanity_band(&env, key, min_wad, max_wad);
    }

    #[only_owner]
    pub fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance) {
        renew_instance(&env);
        admin::set_tolerance(&env, key, tolerance);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    pub fn seed_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        registry::store_oracle(&env, &key, &oracle);
    }

    pub fn remove_oracle(env: Env, key: PriceKey) {
        renew_instance(&env);
        registry::remove_oracle(&env, &key);
    }
}
