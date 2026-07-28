//! Price aggregator — single oracle entry for the lending protocol.
//!
//! # Pipeline
//! ```text
//! price(s) / quote(s)
//!   → Session::new · warm(keys)   // multi-feed bulk by adapter (≥2 feeds)
//!   → for each key: resolve → Outcome   // one evaluator, soft I/O
//!   → force (hard) | to_status (soft)   // only place hard/soft diverge
//! ```
//!
//! Gates (stale / disagree / sanity) live in [`engine`]. All pricing uses
//! [`PriceKey`]; the controller lifts token addresses to `PriceKey::Token`.

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
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};

pub use common::errors::OracleError as Error;

/// Build a session, warm multi-feed leaves under `keys`, run `body`.
fn with_warmed_session(
    env: &Env,
    keys: &Vec<PriceKey>,
    body: impl FnOnce(&mut session::Session),
) {
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
    }

    /// Fail-closed bulk USD prices. Any unsafe key reverts the whole call.
    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve(sess, &key, 0));
            }
        });
        out
    }

    /// Fail-closed single USD price (warms multi-feed leaves for this key).
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

    /// Soft diagnostic for one key (flags; no per-asset revert on stale/disagree).
    pub fn quote(env: Env, key: PriceKey) -> PriceStatus {
        let keys = Vec::from_array(&env, [key.clone()]);
        let mut status = PriceStatus::unusable();
        with_warmed_session(&env, &keys, |sess| {
            status = engine::resolve_status(sess, &key, 0);
        });
        status
    }

    /// Soft bulk diagnostics (one session + multi-feed warm).
    pub fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus> {
        let mut out = Map::new(&env);
        with_warmed_session(&env, &keys, |sess| {
            for key in keys.iter() {
                out.set(key.clone(), engine::resolve_status(sess, &key, 0));
            }
        });
        out
    }

    /// Interval the sources spanned for `key` after gates (low, high) WAD.
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

    /// Stored oracle for `key`, if any.
    pub fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle> {
        admin::get_oracle(&env, &key)
    }

    /// Validate, attest, probe, and store. Owner only.
    #[only_owner]
    pub fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        admin::set_oracle(&env, key, oracle);
    }

    /// Walk sanity band with live containment probe. Owner only.
    #[only_owner]
    pub fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128) {
        admin::set_sanity_band(&env, key, min_wad, max_wad);
    }

    /// Update dual-source agreement band. Owner only.
    #[only_owner]
    pub fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance) {
        admin::set_tolerance(&env, key, tolerance);
    }
}

#[cfg(any(test, feature = "testing"))]
#[contractimpl]
impl PriceAggregator {
    pub fn seed_oracle(env: Env, key: PriceKey, oracle: AssetOracle) {
        admin::store_oracle(&env, &key, &oracle);
    }

    pub fn remove_oracle(env: Env, key: PriceKey) {
        admin::remove_oracle(&env, &key);
    }
}

#[contractimpl]
impl Ownable for PriceAggregator {
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }

    fn renounce_ownership(e: &Env) {
        ownable::renounce_ownership(e);
    }
}
