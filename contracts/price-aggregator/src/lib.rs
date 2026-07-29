//! Single oracle surface for the lending protocol.
//!
//! Keys are [`PriceKey`]. Reads share one `Session` per call: multi-feed leaves
//! are warmed in bulk when a call supplies two or more distinct feed ids on the
//! same adapter, then each key is composed once.
//!
//! Hard paths (`price` / `prices` / `price_spread`) fail closed: any gate or
//! structural failure reverts the whole call. Soft paths (`quote` / `quotes`)
//! return [`PriceStatus`] flags and never panic on market-data problems.
//! Soft and hard share the same compose, blend, and gate evaluation in the
//! engine; only the edge (`force` vs `to_status`) differs.
//!
//! Owner-only writes (`set_oracle`, `set_sanity_band`, `set_tolerance`) validate,
//! attest providers where required, live-probe under hard gates, then store.
//! Ownership is fixed at construct (governance): no transfer, accept, or renounce.

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

/// Build a session, warm multi-feed leaves under `keys`, run `body`.
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
    /// Sets the permanent owner (governance). No oracle config at construction.
    /// Ownership is not transferable or renounceable on this contract.
    pub fn __constructor(env: Env, owner: Address) {
        ownable::set_owner(&env, &owner);
        renew_instance(&env);
    }

    /// Current owner, if set. Always governance after a normal deploy.
    pub fn get_owner(env: Env) -> Option<Address> {
        renew_instance(&env);
        ownable::get_owner(&env)
    }

    /// Fail-closed USD prices for every key in `keys`.
    ///
    /// One session warms multi-feed leaves under the batch, then resolves each
    /// key hard. Any unsafe or unreadable key reverts the entire call.
    ///
    /// # Errors
    /// * [`Error`] variants from compose, blend, and gates (stale, disagree,
    ///   non-positive, sanity, missing config, cycle, depth, factor band, and
    ///   nested quote failures).
    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve(sess, &key, 0));
            }
        });
        out
    }

    /// Fail-closed single USD price for `key`.
    ///
    /// Warms multi-feed leaves reachable from this key, then resolves hard.
    ///
    /// # Errors
    /// * Same gate and structural set as `prices`.
    pub fn price(env: Env, key: PriceKey) -> PriceFeedRaw {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut feed = PriceFeedRaw {
            price_wad: 0,
            asset_decimals: 0,
            timestamp: 0,
        };
        with_warmed_session(&env, &keys, |sess| {
            feed = engine::resolve(sess, &key, 0);
        });
        feed
    }

    /// Soft diagnostic for one key: flags and leg prices without hard revert
    /// on stale, disagree, sanity, or missing market data.
    ///
    /// Structural problems (missing config, cycle, depth, factor band, nested
    /// hard-gate failures during scaled quotes) surface as unusable status.
    pub fn quote(env: Env, key: PriceKey) -> PriceStatus {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut status = PriceStatus::unusable();
        with_warmed_session(&env, &keys, |sess| {
            status = engine::resolve_status(sess, &key, 0);
        });
        status
    }

    /// Soft bulk diagnostics. One session and multi-feed warm for the batch.
    pub fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let mut out = Map::new(&env);
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve_status(sess, &key, 0));
            }
        });
        out
    }

    /// Inclusive WAD interval spanned by dual legs after a hard resolve
    /// (`low`, `high`). Single-source keys return the same price for both.
    ///
    /// Always recomputes (price memo stores the feed only, not the legs).
    ///
    /// # Errors
    /// * Same gate and structural set as `price`.
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

    /// Stored oracle config for `key`, if any.
    pub fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle> {
        renew_instance(&env);
        admin::get_oracle(&env, &key)
    }

    /// Validate, attest providers, live hard-probe, store, and emit update.
    /// Owner only.
    ///
    /// # Errors
    /// * Config validation, provider attestation, and hard probe failures.
    ///
    /// # Events
    /// * `UpdateAssetOracleEvent`
    #[only_owner]
    pub fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        admin::set_oracle(&env, key, oracle);
    }

    /// Replace the sanity band when it overlaps the previous band and the live
    /// hard price sits inside the new band. Owner only.
    ///
    /// # Errors
    /// * [`Error::OracleNotConfigured`] — no stored oracle for `key`.
    /// * Sanity-bound validation and hard probe failures.
    ///
    /// # Events
    /// * `UpdateAssetOracleEvent`
    #[only_owner]
    pub fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128) {
        renew_instance(&env);
        admin::set_sanity_band(&env, key, min_wad, max_wad);
    }

    /// Replace the dual-source agreement band, then hard-probe under the new
    /// band. Owner only.
    ///
    /// # Errors
    /// * [`Error::OracleNotConfigured`] — no stored oracle for `key`.
    /// * Tolerance validation and hard probe failures.
    ///
    /// # Events
    /// * `UpdateAssetOracleEvent`
    #[only_owner]
    pub fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance) {
        renew_instance(&env);
        admin::set_tolerance(&env, key, tolerance);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    /// Test-only: store `oracle` for `key` without validation or probe.
    pub fn seed_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        renew_instance(&env);
        admin::store_oracle(&env, &key, &oracle);
    }

    /// Test-only: delete the stored oracle for `key`.
    pub fn remove_oracle(env: Env, key: PriceKey) {
        renew_instance(&env);
        admin::remove_oracle(&env, &key);
    }
}
