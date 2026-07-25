//! Pins the transaction-local `PriceStatus` memo.
//!
//! The memo is invisible through the ABI — every contract call builds a fresh
//! `ResolutionContext` — so it is pinned here against the context directly.
//! It is what keeps a Reflector leg with a quoted base from re-resolving the
//! same quote once per asset in a `prices` batch: that repricing runs through
//! `resolve_price_status`.
//!
//! Both tests run inside `env.as_contract` because resolving reaches storage
//! for the asset's oracle config.

use super::*;
use crate::PriceAggregator;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

/// A memoized status answers instead of re-resolving. The asset has no oracle
/// config, so an unmemoized resolve would report `unusable` — the sentinel
/// coming back proves the memo was consulted.
#[test]
fn memoized_status_is_returned_without_reresolving() {
    let env = Env::default();
    let id = env.register(PriceAggregator, (Address::generate(&env),));
    let asset = Address::generate(&env);
    let sentinel = PriceStatus {
        final_wad: 7,
        ..PriceStatus::unusable()
    };

    env.as_contract(&id, || {
        let mut cache = ResolutionContext::new(&env);
        cache.store_status(&asset, sentinel.clone());

        assert_eq!(resolve_price_status(&mut cache, &asset), sentinel);
    });
}

/// A resolved status is written to the memo, so the next lookup in the same
/// transaction is a hit rather than another provider read.
#[test]
fn resolved_status_is_written_to_the_memo() {
    let env = Env::default();
    let id = env.register(PriceAggregator, (Address::generate(&env),));
    let asset = Address::generate(&env);

    env.as_contract(&id, || {
        let mut cache = ResolutionContext::new(&env);
        assert!(cache.cached_status(&asset).is_none());

        let resolved = resolve_price_status(&mut cache, &asset);

        assert_eq!(resolved, PriceStatus::unusable());
        assert_eq!(cache.cached_status(&asset), Some(resolved));
    });
}
