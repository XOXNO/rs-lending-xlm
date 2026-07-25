//! Provider dispatch under both read disciplines.
//!
//! `try_read_source` is soft and `read_required_source` is hard, but the split
//! is narrower than the names suggest: soft means *per-asset read problems*
//! become `None`, and nothing more. A config-invariant violation, an asset ref
//! no provider can express, a Reflector contract that reverts at read time, and
//! a quoted reprice whose multiplication overflows all revert straight through
//! the soft read — which is exactly why `compose`'s callers gate each leg
//! before the next one is touched. Those four are the whole set. Every case
//! below is written as a pair where the two disciplines can differ, so a change
//! that collapsed one into the other shows up here rather than in the
//! fail-closed path.
//!
//! Cases whose config carries a quoted base read the quote asset's stored
//! oracle, so they run inside a contract frame via `in_contract`.

use super::*;
use crate::compose::SourceKind;
use crate::storage;
use crate::test_support::{
    in_contract, redstone_single, reflector_quoted, reflector_single, reflector_twap,
    register_redstone_feed, EmptyReflector, EmptyWindowReflector, HugeReflector, PricedReflector,
    RevertingReflector, TwapReflector, TWAP_MEAN_WAD, TWAP_OLDER_AGE_SECS,
};
use common::constants::WAD;
use common::oracle::providers::redstone::RedStonePriceData;
use common::types::OracleAssetRef;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, String, U256};

const NOW: u64 = 1_700_000_000;

/// Ledger clock parked at [`NOW`], so fixture writes land "just now".
fn env_at_now() -> Env {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    env
}

/// The provider family each source config belongs to. The hard path branches on
/// this discriminant to choose between `NoLastPrice` and `InvalidTicker`, so a
/// wrong mapping silently rewrites the error contract.
#[test]
fn source_kind_maps_every_config_variant() {
    let env = env_at_now();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);

    let redstone = redstone_single(&env, &feed, "BTC/USD", 900).primary;
    let OracleSourceConfig::RedStone(ref inner) = redstone else {
        panic!("fixture builds a RedStone primary");
    };
    let xoxno = OracleSourceConfig::Xoxno(inner.clone());
    let reflector_source = reflector_single(&reflector, &asset, 900).primary;

    assert_eq!(SourceKind::of(&redstone), SourceKind::MultiFeed);
    assert_eq!(SourceKind::of(&xoxno), SourceKind::MultiFeed);
    assert_eq!(SourceKind::of(&reflector_source), SourceKind::Reflector);
}

/// A feed id the adapter never published is the canonical per-asset read
/// problem: the soft read reports it as `None`.
#[test]
fn try_read_source_returns_none_for_a_missing_multi_feed() {
    let env = env_at_now();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let source = redstone_single(&env, &feed, "MISSING", 900).primary;
    let mut cache = ResolutionContext::new(&env);

    assert!(try_read_source(&mut cache, &source).is_none());
}

/// The hard read of the same source reverts with the multi-feed family's error.
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn read_required_source_reverts_invalid_ticker_for_a_missing_multi_feed() {
    let env = env_at_now();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let source = redstone_single(&env, &feed, "MISSING", 900).primary;
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// A Xoxno source reads through the same multi-feed adapter path as RedStone —
/// the two share a wire ABI and differ only in the variant name.
#[test]
fn xoxno_source_reads_through_the_multi_feed_path() {
    let env = env_at_now();
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &(3 * WAD));
    let OracleSourceConfig::RedStone(inner) = redstone_single(&env, &feed, "BTC/USD", 900).primary
    else {
        panic!("fixture builds a RedStone primary");
    };
    let source = OracleSourceConfig::Xoxno(inner);
    let mut cache = ResolutionContext::new(&env);

    let observation = try_read_source(&mut cache, &source).expect("xoxno source readable");

    assert_eq!(observation.price_wad, 3 * WAD);
}

/// A missing Xoxno feed raises the multi-feed family's error, not the
/// Reflector one. Both arms of the hard dispatch's error choice compile either
/// way round, so this is what holds the Xoxno half in place.
#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn read_required_source_reverts_invalid_ticker_for_a_missing_xoxno_feed() {
    let env = env_at_now();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let OracleSourceConfig::RedStone(inner) = redstone_single(&env, &feed, "MISSING", 900).primary
    else {
        panic!("fixture builds a RedStone primary");
    };
    let source = OracleSourceConfig::Xoxno(inner);
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// A Reflector reporting no last price is the same per-asset problem, softened
/// the same way.
#[test]
fn try_read_source_returns_none_for_a_reflector_with_no_last_price() {
    let env = env_at_now();
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_single(&reflector, &asset, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    assert!(try_read_source(&mut cache, &source).is_none());
}

/// The hard read of that Reflector source reverts with the Reflector family's
/// error, distinct from the multi-feed one above.
#[test]
#[should_panic(expected = "Error(Contract, #210)")]
fn read_required_source_reverts_no_last_price_for_an_empty_reflector() {
    let env = env_at_now();
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_single(&reflector, &asset, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// On a readable feed the two disciplines produce the same observation. The
/// hard read is a replay of what the soft read already did, so a divergence
/// here would mean the fail-closed path prices off different numbers than the
/// diagnostic view reports.
#[test]
fn both_disciplines_agree_on_a_readable_feed() {
    let env = env_at_now();
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "BTC/USD"), &(3 * WAD));
    let source = redstone_single(&env, &feed, "BTC/USD", 900).primary;
    let mut cache = ResolutionContext::new(&env);

    let soft = try_read_source(&mut cache, &source).expect("feed readable");
    let hard = read_required_source(&mut cache, &source);

    assert_eq!(soft.price_wad, 3 * WAD);
    assert_eq!(hard.price_wad, soft.price_wad);
    assert_eq!(hard.timestamp(), soft.timestamp());
    assert_eq!(soft.timestamp(), NOW);
}

/// A payload warmed into the transaction cache is used instead of a fresh
/// adapter call. The adapter never published this feed id, so a read that
/// bypassed the cache would report `None`.
#[test]
fn a_warmed_bulk_entry_is_read_instead_of_the_adapter() {
    let env = env_at_now();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let feed_id = String::from_str(&env, "BTC/USD");
    let source = redstone_single(&env, &feed, "BTC/USD", 900).primary;
    let mut cache = ResolutionContext::new(&env);
    // 3 WAD at the source's 8 decimals.
    cache.set_bulk_feed(
        &feed,
        &feed_id,
        RedStonePriceData {
            price: U256::from_u128(&env, 300_000_000),
            package_timestamp: NOW * 1_000,
            write_timestamp: NOW * 1_000,
        },
    );

    let observation = try_read_source(&mut cache, &source).expect("warmed entry readable");

    assert_eq!(observation.price_wad, 3 * WAD);
}

/// A TWAP window averages the history it was given and dates itself to the
/// oldest sample. The fixture's samples differ in both price and timestamp, so
/// neither a single sample echoed back nor the newest timestamp can pass.
#[test]
fn twap_read_averages_the_history_and_dates_it_to_the_oldest_sample() {
    let env = env_at_now();
    let reflector = env.register(TwapReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 2, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    let soft = try_read_source(&mut cache, &source).expect("history long enough");
    let hard = read_required_source(&mut cache, &source);

    assert_eq!(soft.price_wad, TWAP_MEAN_WAD);
    assert_eq!(soft.timestamp(), NOW - TWAP_OLDER_AGE_SECS);
    assert_eq!(hard.price_wad, soft.price_wad);
    assert_eq!(hard.timestamp(), soft.timestamp());
}

/// A window with no history at all is a per-asset read problem, so the soft
/// read softens it.
#[test]
fn try_read_source_softens_an_empty_twap_history() {
    let env = env_at_now();
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 4, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    assert!(try_read_source(&mut cache, &source).is_none());
}

/// The hard read names it. This is the provider answering with no history
/// object at all; the case below is the other branch of the same rejection.
#[test]
#[should_panic(expected = "Error(Contract, #212)")]
fn read_required_source_reverts_reflector_history_empty() {
    let env = env_at_now();
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 4, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// A history the provider did return, holding no samples, is the same
/// rejection. It has to be pinned separately: the emptiness check sits after
/// the absent-history check, so the case above never reaches it, and a window
/// of zero samples is also shorter than any observation minimum — leaving this
/// branch free to raise the short-history error instead without any existing
/// case noticing.
#[test]
#[should_panic(expected = "Error(Contract, #212)")]
fn read_required_source_reverts_history_empty_for_a_present_but_empty_window() {
    let env = env_at_now();
    let reflector = env.register(EmptyWindowReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 4, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// History present but shorter than the window needs is also per-asset, and
/// also softened. The fixture returns two samples against a twelve-record
/// window, which asks for six.
#[test]
fn try_read_source_softens_a_short_twap_history() {
    let env = env_at_now();
    let reflector = env.register(TwapReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 12, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    assert!(try_read_source(&mut cache, &source).is_none());
}

/// The hard read distinguishes a short history from an absent one.
#[test]
#[should_panic(expected = "Error(Contract, #219)")]
fn read_required_source_reverts_twap_insufficient_observations() {
    let env = env_at_now();
    let reflector = env.register(TwapReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 12, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    read_required_source(&mut cache, &source);
}

/// Soft does not mean panic-free. A zero-record window is a broken config, not
/// a feed that happens to be unavailable, and `validate_twap_records` rejects
/// it in both disciplines — before any provider call. The fixture's history
/// would satisfy a zero-record window's observation minimum, so without that
/// guard this read would succeed rather than merely soften.
#[test]
#[should_panic(expected = "Error(Contract, #219)")]
fn try_read_source_reverts_on_a_zero_record_twap_window() {
    let env = env_at_now();
    let reflector = env.register(TwapReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 0, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    try_read_source(&mut cache, &source);
}

/// The other half of the same config-invariant guard, with an error code no
/// other path raises: a window above the record cap reverts the soft read too.
#[test]
#[should_panic(expected = "Error(Contract, #228)")]
fn try_read_source_reverts_on_an_over_cap_twap_window() {
    let env = env_at_now();
    let reflector = env.register(TwapReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_twap(&reflector, &asset, 13, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    try_read_source(&mut cache, &source);
}

/// An asset ref no SEP-40 provider can express reverts the soft read as well:
/// `to_reflector_asset` has no `None` to return.
#[test]
#[should_panic(expected = "Error(Contract, #204)")]
fn try_read_source_reverts_on_a_string_asset_ref() {
    let env = env_at_now();
    let reflector = env.register(EmptyReflector, ());
    let asset = Address::generate(&env);
    let mut source = reflector_single(&reflector, &asset, 900).primary;
    let OracleSourceConfig::Reflector(ref mut config) = source else {
        panic!("fixture builds a Reflector primary");
    };
    config.asset = OracleAssetRef::String(String::from_str(&env, "BTC"));
    let mut cache = ResolutionContext::new(&env);

    try_read_source(&mut cache, &source);
}

/// A Reflector contract that reverts at read time — paused, archived, or
/// upgraded past the interface — takes the soft read down with it: the client
/// call is not a `try_` call, so there is nothing to soften. This is why the
/// status path stops its traversal at the first unreadable leg instead of
/// reading an anchor it has already decided to ignore.
#[test]
#[should_panic(expected = "Error(Contract, #216)")]
fn try_read_source_reverts_when_the_reflector_contract_reverts() {
    let env = env_at_now();
    let reflector = env.register(RevertingReflector, ());
    let asset = Address::generate(&env);
    let source = reflector_single(&reflector, &asset, 900).primary;
    let mut cache = ResolutionContext::new(&env);

    try_read_source(&mut cache, &source);
}

/// A quoted base whose quote asset has no oracle of its own is a per-asset
/// problem the soft read absorbs, after the token leg itself read cleanly.
#[test]
fn try_read_source_softens_an_unresolvable_quote_leg() {
    let env = env_at_now();
    let reflector = env.register(PricedReflector, ());
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let source = reflector_quoted(&reflector, &asset, &quote, 900).primary;

    let observation = in_contract(&env, || {
        let mut cache = ResolutionContext::new(&env);
        try_read_source(&mut cache, &source)
    });

    assert!(observation.is_none());
}

/// The hard read of the same source names the quote as the problem rather than
/// reporting the token leg unreadable.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn read_required_source_reverts_invalid_oracle_base_for_an_unresolvable_quote() {
    let env = env_at_now();
    let reflector = env.register(PricedReflector, ());
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let source = reflector_quoted(&reflector, &asset, &quote, 900).primary;

    in_contract(&env, || {
        let mut cache = ResolutionContext::new(&env);
        read_required_source(&mut cache, &source)
    });
}

/// The fourth way the soft read reverts, and the only one that fires after
/// every provider call has already returned: the reprice multiplication itself
/// overflows. Both legs read cleanly — positive, in scale, upscaling to WAD
/// without loss — and normalization has no complaint about either, because it
/// judges one price at a time. Their product is what does not fit, and
/// `Wad::mul` reverts rather than saturating, so there is nothing for the soft
/// discipline to turn into `None`.
///
/// Both legs read from one stub, so both price at `1e29` WAD and the product is
/// `1e40` against an `i128` ceiling near `1.7e38`. Sharing the stub is what
/// forces the widened band below, and that widening is a fixture convenience,
/// not what makes the overflow possible: an asymmetric pair reaches it inside
/// bounds `validate_oracle_config` accepts, since a token leg is bounded only
/// by the normalizer while `MAX_REASONABLE_PRICE_WAD` caps the quote — `1e30`
/// against a quote at `9e26` already exceeds the ceiling.
#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn try_read_source_reverts_on_a_quoted_reprice_overflow() {
    let env = env_at_now();
    let reflector = env.register(HugeReflector, ());
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let source = reflector_quoted(&reflector, &asset, &quote, 900).primary;
    // A quote leg backs a reprice only while its status is VALID, and the
    // fixture band refuses a price this large. Seeded straight into storage, so
    // the band is opened rather than the shared stub's price lowered.
    let mut quote_config = reflector_single(&reflector, &quote, 900);
    quote_config.max_sanity_price_wad = i128::MAX;

    in_contract(&env, || {
        storage::set_oracle_config(&env, &quote, &quote_config);
        let mut cache = ResolutionContext::new(&env);
        try_read_source(&mut cache, &source);
    });
}
