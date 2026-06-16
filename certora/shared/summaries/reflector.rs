//! Reflector oracle summaries: positive prices and bounded timestamps.

use cvlr::cvlr_assume;
use cvlr::nondet::{nondet, nondet_option};
use soroban_sdk::{Address, Env, Symbol, Vec};

use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

const MAX_CLOCK_SKEW_SECS: u64 = 60;
const MAX_PRICES_LEN: u32 = 20;

pub fn base_summary(env: &Env, _oracle: &Address) -> ReflectorAsset {
    ReflectorAsset::Other(Symbol::new(env, "USD"))
}

/// Last price: `price > 0`, timestamp within ledger + skew.
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

/// Historical prices: positive entries, descending timestamps, length capped at `records`.
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
            cvlr_assume!(timestamp <= prev_ts);
            out.push_back(ReflectorPriceData { price, timestamp });
            prev_ts = timestamp;
        }
        out
    })
}
