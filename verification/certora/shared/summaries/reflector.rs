//! Summaries for the SEP-40 Reflector oracle ABI.
//!
//! The Reflector trait is declared in `controller/src/oracle/reflector.rs`
//! via `#[contractclient(name = "ReflectorClient")]`. Production goes through
//! `crate::oracle::token_price` -> `ReflectorClient::lastprice` /
//! `ReflectorClient::prices`; these summaries constrain oracle outputs to the
//! same domain expected by production price validation.
//!
//! Soundness contract: each summary returns a value in the same domain as
//! the production Reflector contract guarantees (price > 0, timestamps
//! bounded by current ledger time + 60s clock-skew tolerance, monotone
//! decreasing for `prices`). Anything stricter would silently hide feasible
//! behavior.
//!
//! Wiring: registered against `ReflectorClient::*` call sites via
//! `cvlr_soroban_macros::apply_summary!`.

use cvlr::cvlr_assume;
use cvlr::nondet::{nondet, nondet_option};
use soroban_sdk::{Address, Env, Vec};

use crate::oracle::reflector::{ReflectorAsset, ReflectorPriceData};

// ---------------------------------------------------------------------------
// Bounds applied by the production staleness / sanity checks
// ---------------------------------------------------------------------------

/// Maximum clock skew tolerated by `crate::oracle::check_not_future`
/// (60 seconds). Feed timestamps further in the future panic with
/// `PriceFeedStale`.
const MAX_CLOCK_SKEW_SECS: u64 = 60;

/// Maximum length of the historical prices Vec the prover is allowed to
/// reason over. Reflector's `prices` accepts an arbitrary `records` count;
/// bounding to 20 keeps Vec unrolling tractable while comfortably covering
/// every rule's read window (the largest production caller asks for the
/// last few entries to compute a TWAP / median).
const MAX_PRICES_LEN: u32 = 20;

// ---------------------------------------------------------------------------
// `lastprice`
// ---------------------------------------------------------------------------

/// Summary for `ReflectorClient::lastprice`.
///
/// Production guarantees (SEP-40 + controller-side post-conditions):
///   * Returns `None` when the asset is not configured in the oracle.
///   * Returns `Some(ReflectorPriceData { price, timestamp })` with
///     `price > 0` (production code panics with `InvalidPrice` on a
///     non-positive feed; modelling this ahead of the call removes a sink
///     branch for the prover).
///   * `timestamp <= ledger().timestamp() + 60` -- the clock-skew gate at
///     `controller/src/oracle/mod.rs::check_not_future` rejects further-out
///     timestamps.
pub fn lastprice_summary(
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
// `prices`
// ---------------------------------------------------------------------------

/// Summary for `ReflectorClient::prices`.
///
/// Production guarantees:
///   * Returns `None` when the asset is not configured.
///   * On `Some`, returns up to `records` entries -- each entry is one
///     historical snapshot. Entries are ordered most-recent-first
///     (`prices[0].timestamp >= prices[1].timestamp >= ...`).
///   * Each `price > 0`; each `timestamp` bounded above by current ledger
///     time + 60s clock-skew tolerance.
///
/// Bounded length: `records.min(MAX_PRICES_LEN)`. The cap is a verification
/// hygiene constraint -- without it the prover would unroll an unbounded
/// Vec construction loop. Production has no formal cap; rules that need a
/// specific length should constrain `records` themselves.
pub fn prices_summary(
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
            // Monotone non-increasing timestamp chain. Each sample is bounded
            // by the preceding newer entry's timestamp.
            cvlr_assume!(timestamp <= prev_ts);
            out.push_back(ReflectorPriceData { price, timestamp });
            prev_ts = timestamp;
        }
        out
    })
}
