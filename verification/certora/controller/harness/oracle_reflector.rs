//! Certora harness substitute for `controller::oracle::reflector`.
//!
//! Under `--features certora`, this file replaces the production
//! module so the prover sees nondet-bounded values for the SEP-40
//! cross-contract reads. Without summaries every rule that touches a
//! price feed becomes vacuous over havoced cross-contract returns.
//!
//! Re-exports the production types (`ReflectorAsset`, `ReflectorPriceData`,
//! `ReflectorClient`) unchanged — only the three `reflector_*_call`
//! wrapper functions are replaced.

#[allow(dead_code)]
#[path = "../../../../controller/src/oracle/reflector.rs"]
mod __prod;

pub use __prod::{ReflectorAsset, ReflectorClient, ReflectorPriceData};

use cvlr::cvlr_assume;
use cvlr::nondet::{nondet, nondet_option};
use soroban_sdk::{Address, Env, Symbol, Vec};

// Production staleness gate (60 s). Feed timestamps further in the
// future are rejected by `oracle::check_not_future`.
const MAX_CLOCK_SKEW_SECS: u64 = 60;

// Cap on prover-visible Vec length for `prices`. Production accepts
// any `records`; bounding to 20 keeps Vec unrolling tractable while
// covering every live TWAP/median read window.
const MAX_PRICES_LEN: u32 = 20;

// ---------------------------------------------------------------------------
// Summary for `ReflectorClient::base`
// ---------------------------------------------------------------------------

pub(crate) fn reflector_base_call(env: &Env, _oracle: &Address) -> ReflectorAsset {
    ReflectorAsset::Other(Symbol::new(env, "USD"))
}

// ---------------------------------------------------------------------------
// Summary for `ReflectorClient::lastprice`
// ---------------------------------------------------------------------------
//
// SEP-40 + controller post-conditions:
//   * `None` when the asset is not configured.
//   * `Some(ReflectorPriceData { price, timestamp })` with `price > 0`
//     (production panics on non-positive feeds).
//   * `timestamp <= ledger().timestamp() + 60` (clock-skew gate).
pub(crate) fn reflector_lastprice_call(
    env: &Env,
    _oracle: &Address,
    _asset: &ReflectorAsset,
) -> Option<ReflectorPriceData> {
    nondet_option(|| {
        let price: i128 = nondet();
        let timestamp: u64 = nondet();
        cvlr_assume!(price > 0);
        cvlr_assume!(timestamp <= env.ledger().timestamp() + MAX_CLOCK_SKEW_SECS);
        ReflectorPriceData { price, timestamp }
    })
}

// ---------------------------------------------------------------------------
// Summary for `ReflectorClient::prices`
// ---------------------------------------------------------------------------
//
// Production guarantees:
//   * `None` when the asset is not configured.
//   * On `Some`, up to `records` snapshots ordered most-recent-first:
//     `prices[0].timestamp >= prices[1].timestamp >= ...`.
//   * Each `price > 0`; each `timestamp` bounded by ledger time + skew.
//
// Length capped at `MAX_PRICES_LEN = 20` to keep loop unrolling tractable.
pub(crate) fn reflector_prices_call(
    env: &Env,
    _oracle: &Address,
    _asset: &ReflectorAsset,
    records: u32,
) -> Option<Vec<ReflectorPriceData>> {
    nondet_option(|| {
        let len: u32 = if records > MAX_PRICES_LEN {
            MAX_PRICES_LEN
        } else {
            records
        };
        let mut out: Vec<ReflectorPriceData> = Vec::new(env);
        let now_plus_skew = env.ledger().timestamp() + MAX_CLOCK_SKEW_SECS;
        let mut prev_ts: u64 = now_plus_skew;
        for _ in 0..len {
            let price: i128 = nondet();
            let timestamp: u64 = nondet();
            cvlr_assume!(price > 0);
            // Monotone non-increasing timestamps within the window.
            cvlr_assume!(timestamp <= prev_ts);
            out.push_back(ReflectorPriceData { price, timestamp });
            prev_ts = timestamp;
        }
        out
    })
}
