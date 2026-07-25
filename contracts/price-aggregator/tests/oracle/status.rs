//! Pins the transaction-local `PriceStatus` memo, the soft path's agreement
//! with the hard path, and the blend fields only the soft path reports.
//!
//! The memo is invisible through the ABI — every contract call builds a fresh
//! `ResolutionContext` — so it is pinned here against the context directly.
//! It is what keeps a Reflector leg with a quoted base from re-resolving the
//! same quote once per asset in a `prices` batch: that repricing runs through
//! `resolve_price_status`.
//!
//! The memo tests run inside `env.as_contract` because resolving reaches
//! storage for the asset's oracle config. The rest drive the deployed contract,
//! because agreement is a claim about the two public entry points.

use super::*;
use common::constants::WAD;
use common::types::{OracleSourceConfigOption, OracleStrategy};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Env, String};

use crate::test_support::{
    redstone_dual, redstone_primary_reflector_anchor, redstone_single, reflector_quoted,
    reflector_single, register_redstone_feed, EmptyReflector, PricedReflector, RevertingReflector,
};
use crate::{PriceAggregator, PriceAggregatorClient};

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

const NOW: u64 = 1_700_000_000;

/// Ledger clock parked at `NOW`, so a fixture's feed writes land "just now"
/// unless it backdates them on purpose.
fn env_at_now() -> Env {
    let env = Env::default();
    env.ledger().with_mut(|li| li.timestamp = NOW);
    env
}

fn register_agg(env: &Env) -> PriceAggregatorClient<'_> {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    PriceAggregatorClient::new(env, &id)
}

/// The invariant the shared `Composition` exists to hold: `valid` says exactly
/// whether `price` would have produced a number.
///
/// It is load-bearing beyond the view. `providers::reflector` reprices a quoted
/// base only through a `valid` status, so a soft path that accepted more than
/// the hard path would let a fail-closed price rest on a leg `price` rejects.
fn assert_agrees(label: &str, client: &PriceAggregatorClient<'_>, asset: &Address) {
    assert_eq!(
        client.price_status(asset).valid,
        client.try_price(asset).is_ok(),
        "soft/hard disagreement for fixture: {label}"
    );
}

/// One fixture per row of the hard-path failure table, plus the two shapes that
/// succeed. When the two paths each re-implemented staleness, band, and
/// strategy dispatch, either could drift; rendering both from one `Composition`
/// is what makes this test more than a snapshot.
#[test]
fn valid_status_implies_hard_path_succeeds() {
    // Success (single): fresh primary inside the sanity band.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

        // Anchors the test against vacuity: something must be valid.
        assert!(client.price_status(&asset).valid);
        assert_agrees("single fresh", &client, &asset);
    }

    // Success (dual): both legs fresh and inside the tolerance band.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
        feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD + WAD / 50));
        client.seed_oracle_config(
            &asset,
            &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
        );

        assert!(client.price_status(&asset).valid);
        assert_agrees("dual in band", &client, &asset);
    }

    // Success (quoted base): a Reflector primary priced through a USD-rooted
    // quote. The only shape that consumes `valid` on the fail-closed path, so
    // an agreement test without it pins the invariant everywhere except where
    // it is load-bearing.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let quote = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "QUOTE/USD"), &WAD);
        client.seed_oracle_config(&quote, &redstone_single(&env, &feed, "QUOTE/USD", 900));
        let reflector = env.register(PricedReflector, ());
        client.seed_oracle_config(&asset, &reflector_quoted(&reflector, &asset, &quote, 900));

        assert!(client.price_status(&asset).valid);
        assert_agrees("quoted base", &client, &asset);
    }

    // Quoted base whose quote leg is unusable (quote feed id never populated):
    // the reprice has nothing to rest on, so both paths must refuse it.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let quote = Address::generate(&env);
        let (feed, _feed_client) = register_redstone_feed(&env);
        client.seed_oracle_config(&quote, &redstone_single(&env, &feed, "MISSING", 900));
        let reflector = env.register(PricedReflector, ());
        client.seed_oracle_config(&asset, &reflector_quoted(&reflector, &asset, &quote, 900));

        assert_agrees("quoted base, unusable quote", &client, &asset);
    }

    // No `AssetOracle` at all.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        assert_agrees("unconfigured", &client, &Address::generate(&env));
    }

    // Row 1: config is the `pending_for` self-pointer sentinel.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        client.seed_oracle_config(&asset, &AssetOracleConfig::pending_for(asset.clone(), 7));

        assert_agrees("pending config", &client, &asset);
    }

    // Row 2 (Reflector): primary source unreadable.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let reflector = env.register(EmptyReflector, ());
        client.seed_oracle_config(&asset, &reflector_single(&reflector, &asset, 900));

        assert_agrees("reflector primary unreadable", &client, &asset);
    }

    // Row 2 (RedStone/Xoxno): primary feed id never populated.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, _feed_client) = register_redstone_feed(&env);
        client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "MISSING", 900));

        assert_agrees("redstone primary unreadable", &client, &asset);
    }

    // Row 3: primary older than its max-stale window.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 60));
        env.ledger().with_mut(|li| li.timestamp = NOW + 10_000);

        assert_agrees("primary stale", &client, &asset);
    }

    // Row 4: strategy is dual but `config.anchor` is `None`.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
        cfg.strategy = OracleStrategy::PrimaryWithAnchor;
        cfg.anchor = OracleSourceConfigOption::None;
        client.seed_oracle_config(&asset, &cfg);

        assert_agrees("dual without anchor config", &client, &asset);
    }

    // Row 5: anchor configured but its feed id was never populated.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
        client.seed_oracle_config(
            &asset,
            &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
        );

        assert_agrees("anchor unreadable", &client, &asset);
    }

    // Row 5: anchor readable but past its max-stale window.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
        let stale_ms = (NOW - 10_000) * 1_000;
        feed_client.set_price_data(
            &String::from_str(&env, "ANCHOR"),
            &WAD,
            &stale_ms,
            &stale_ms,
        );
        client.seed_oracle_config(
            &asset,
            &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
        );

        assert_agrees("anchor stale", &client, &asset);
    }

    // Row 6: primary and anchor outside the tolerance band.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
        feed_client.set_price(&String::from_str(&env, "ANCHOR"), &(WAD * 2));
        client.seed_oracle_config(
            &asset,
            &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
        );

        assert_agrees("dual out of band", &client, &asset);
    }

    // Row 7: provider payload carries a non-positive price.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price_data(
            &String::from_str(&env, "BTC/USD"),
            &0i128,
            &(NOW * 1_000),
            &(NOW * 1_000),
        );
        client.seed_oracle_config(&asset, &redstone_single(&env, &feed, "BTC/USD", 900));

        assert_agrees("non-positive payload", &client, &asset);
    }

    // Row 8: final price outside the sanity band.
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
        cfg.min_sanity_price_wad = WAD * 10;
        cfg.max_sanity_price_wad = WAD * 20;
        client.seed_oracle_config(&asset, &cfg);

        assert_agrees("outside sanity band", &client, &asset);
    }

    // Row 8: sanity band disabled (`max_sanity_price_wad <= 0`).
    {
        let env = env_at_now();
        let client = register_agg(&env);
        let asset = Address::generate(&env);
        let (feed, feed_client) = register_redstone_feed(&env);
        feed_client.set_price(&String::from_str(&env, "BTC/USD"), &WAD);
        let mut cfg = redstone_single(&env, &feed, "BTC/USD", 900);
        cfg.max_sanity_price_wad = 0;
        client.seed_oracle_config(&asset, &cfg);

        assert_agrees("sanity band disabled", &client, &asset);
    }
}

/// An unreadable primary settles the answer, so the anchor is never read — and
/// here that matters, because reading it would revert.
///
/// Nothing in this config is invalid: a RedStone primary with a Reflector spot
/// anchor is a shape `set_oracle_config` and the governance probe both accept.
/// Only the runtime fails — the primary feed id is absent and the anchor's
/// Reflector contract reverts, as a paused, archived, or upgraded one would. A
/// diagnostic view that touched the anchor anyway would inherit that revert
/// instead of reporting `unusable`.
#[test]
fn unreadable_primary_leaves_a_reverting_anchor_unread() {
    let env = env_at_now();
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    // "MISSING" is never populated on the mock feed.
    let (feed, _feed_client) = register_redstone_feed(&env);
    let reflector = env.register(RevertingReflector, ());
    client.seed_oracle_config(
        &asset,
        &redstone_primary_reflector_anchor(&env, &feed, "MISSING", &reflector, &asset, 900),
    );

    assert_eq!(
        client.try_price_status(&asset),
        Ok(Ok(PriceStatus::unusable()))
    );
    assert_agrees("unreadable primary, reverting anchor", &client, &asset);
}

/// An agreeing dual pair reports the older leg's timestamp, not the primary's.
/// The `price_timestamp` of a blend has no hard-path counterpart to compare
/// against, so nothing but this pins which leg it comes from.
#[test]
fn dual_in_band_status_reports_the_older_leg_timestamp() {
    let env = env_at_now();
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    let older = NOW - 500;
    let anchor_wad = WAD + WAD / 50;
    feed_client.set_price_data(
        &String::from_str(&env, "ANCHOR"),
        &anchor_wad,
        &(older * 1_000),
        &(older * 1_000),
    );
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);

    assert!(status.valid);
    assert!(!status.deviation);
    assert_eq!(status.price_timestamp, older);
    assert_eq!(status.final_wad, (WAD + anchor_wad) / 2);
    assert_eq!(status.secondary_wad, anchor_wad);
}

/// A disagreeing dual pair reports the same midpoint and older-leg timestamp
/// the agreeing case would have, and flags `deviation` instead of hiding them.
/// The out-of-band branch computes both itself, so it needs its own lock.
#[test]
fn dual_out_of_band_status_reports_the_older_leg_timestamp() {
    let env = env_at_now();
    let client = register_agg(&env);
    let asset = Address::generate(&env);
    let (feed, feed_client) = register_redstone_feed(&env);
    feed_client.set_price(&String::from_str(&env, "PRIMARY"), &WAD);
    let older = NOW - 500;
    feed_client.set_price_data(
        &String::from_str(&env, "ANCHOR"),
        &(WAD * 2),
        &(older * 1_000),
        &(older * 1_000),
    );
    client.seed_oracle_config(
        &asset,
        &redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500),
    );

    let status = client.price_status(&asset);

    assert!(!status.valid);
    assert!(status.deviation);
    assert!(!status.stale);
    assert_eq!(status.price_timestamp, older);
    assert_eq!(status.final_wad, (WAD + WAD * 2) / 2);
    assert_eq!(status.secondary_wad, WAD * 2);
}
