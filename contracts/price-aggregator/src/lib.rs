#![no_std]

mod admin;
mod engine;
mod observation;
mod properties;
mod providers;
mod session;
mod tolerance;

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

fn with_warmed_session(env: &Env, keys: &Vec<PriceKey>, body: impl FnOnce(&mut session::Session)) {
    renew_instance(env);
    let mut sess = session::Session::new(env);
    sess.warm(keys);
    body(&mut sess);
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
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve(sess, &key, 0));
            }
        });
        out
    }

    pub fn price(env: Env, key: PriceKey) -> PriceFeedRaw {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut feed = PriceFeedRaw {
            price_wad: 0,
            low_wad: 0,
            high_wad: 0,
            asset_decimals: 0,
            timestamp: 0,
        };
        with_warmed_session(&env, &keys, |sess| {
            feed = engine::resolve(sess, &key, 0);
        });
        feed
    }

    pub fn quote(env: Env, key: PriceKey) -> PriceStatus {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut status = PriceStatus::unusable();
        with_warmed_session(&env, &keys, |sess| {
            status = engine::resolve_status(sess, &key, 0);
        });
        status
    }

    pub fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let mut out = Map::new(&env);
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve_status(sess, &key, 0));
            }
        });
        out
    }

    pub fn price_spread(env: Env, key: PriceKey) -> (i128, i128) {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut low = 0;
        let mut high = 0;
        with_warmed_session(&env, &keys, |sess| {
            let (_, outcome) = engine::resolve_detailed(sess, &key, 0);
            low = outcome.low_wad;
            high = outcome.high_wad;
        });
        (low, high)
    }

    pub fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle> {
        renew_instance(&env);
        admin::get_oracle(&env, &key)
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
        admin::store_oracle(&env, &key, &oracle);
    }

    pub fn remove_oracle(env: Env, key: PriceKey) {
        renew_instance(&env);
        admin::remove_oracle(&env, &key);
    }
}
