//! `validate_oracle_config` is the last gate an owner call passes before an
//! `AssetOracle` becomes live, and the only one that runs on a direct call to
//! the aggregator. Governance validates richer inputs upstream, but nothing
//! forces an owner to arrive through governance, so what this function rejects
//! is what the aggregator actually refuses to store.
//!
//! Each case takes a fixture that validates cleanly and breaks exactly one
//! rule, so the error it raises names the rule that caught it. Two cases run
//! the other way: they pin bounds this function does *not* apply.
//!
//! Quote-base validation reads the quote asset's own stored config, so the
//! cases that reach it run inside a contract frame via [`in_contract`].

use super::*;
use crate::test_support::{
    redstone_dual, redstone_primary_reflector_anchor, redstone_single, reflector_quoted,
    reflector_single, reflector_twap, register_redstone_feed,
};
use crate::PriceAggregator;
use common::constants::WAD;
use common::types::OracleSourceConfigOption;
use soroban_sdk::testutils::Address as _;

/// Runs `body` in a contract frame, which the quote-oracle lookup requires.
fn in_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

/// The baseline every rejection case below mutates: a single-source RedStone
/// config with an ordered, non-pinched, single-source-width sanity band.
#[test]
fn well_formed_single_source_config_is_accepted() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// The dual baseline: a wide sanity band the single-source cap would refuse,
/// carried by an anchored strategy that is allowed it, plus a tolerance inside
/// the envelope.
#[test]
fn well_formed_anchored_config_is_accepted() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// An inverted band is rejected before anything else looks at it.
#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn inverted_sanity_band_reverts_invalid_sanity_bounds() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let mut config = redstone_single(&env, &feed, "BTC/USD", 900);
    config.min_sanity_price_wad = WAD * 2;
    config.max_sanity_price_wad = WAD;

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// A band ordered correctly but pinched around par is rejected too: it would
/// pass the first read and then brick every one after it, as the price moves a
/// basis point.
#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn pinched_sanity_band_reverts_invalid_sanity_bounds() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let mut config = redstone_single(&env, &feed, "BTC/USD", 900);
    config.min_sanity_price_wad = WAD;
    config.max_sanity_price_wad = WAD + WAD / 10_000;

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// The same band the anchored baseline is allowed becomes a rejection once the
/// strategy says one source. Only the strategy changes, so nothing but the
/// strategy-dependent width cap can be what caught it.
#[test]
#[should_panic(expected = "Error(Contract, #226)")]
fn wide_band_under_single_strategy_reverts_band_too_wide() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let mut config = redstone_dual(&env, &feed, "PRIMARY", "ANCHOR", 900, 10_500, 9_500);
    config.strategy = OracleStrategy::Single;

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// The single-source fixture carries a degenerate tolerance (par on both legs,
/// below the minimum envelope) and validates anyway: a `Single` config never
/// consults the band, so requiring one would reject configs that are fine.
/// Pairs with the case below, where the identical tolerance is rejected.
#[test]
fn single_strategy_does_not_validate_the_unused_tolerance() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let config = redstone_single(&env, &feed, "BTC/USD", 900);
    assert_eq!(config.tolerance.upper_ratio_bps, 10_000);
    assert_eq!(config.tolerance.lower_ratio_bps, 10_000);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// The same degenerate tolerance under an anchored strategy is rejected: the
/// band is the whole point of a second source, and a par-to-par band would let
/// any two prices agree.
#[test]
#[should_panic(expected = "Error(Contract, #208)")]
fn anchored_strategy_reverts_bad_last_tolerance() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let mut config = redstone_single(&env, &feed, "BTC/USD", 900);
    config.strategy = OracleStrategy::PrimaryWithAnchor;

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// An asset quoted in itself is rejected at config time rather than left to the
/// read-time cycle guard.
///
/// The asset is given its own USD-rooted oracle first, so the quote lookup that
/// follows the self-quote check would accept it: nothing but the self-quote
/// rule can be what rejected this config.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn self_quoted_reflector_base_reverts_invalid_oracle_base() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let config = reflector_quoted(&reflector, &asset, &asset, 900);

    in_contract(&env, || {
        storage::set_oracle_config(&env, &asset, &reflector_single(&reflector, &asset, 900));
        validate_oracle_config(&env, &asset, &config);
    });
}

/// A quote asset with no `AssetOracle` of its own is rejected: there would be
/// nothing to reprice the quoted leg through.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn unconfigured_quote_reverts_invalid_oracle_base() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let config = reflector_quoted(&reflector, &asset, &quote, 900);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// A quote that is itself quoted in a third asset is rejected: quoting is one
/// hop to USD, never a chain.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn quote_chain_reverts_invalid_oracle_base() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let second_hop = Address::generate(&env);
    let config = reflector_quoted(&reflector, &asset, &quote, 900);

    in_contract(&env, || {
        storage::set_oracle_config(
            &env,
            &quote,
            &reflector_quoted(&reflector, &quote, &second_hop, 900),
        );
        validate_oracle_config(&env, &asset, &config);
    });
}

/// A quote with its own USD-rooted oracle is what the rule asks for, so the
/// quoted config validates. Without this, every rejection above would also pass
/// against a validator that refused quoted bases outright.
#[test]
fn quoted_base_with_a_usd_rooted_quote_is_accepted() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let config = reflector_quoted(&reflector, &asset, &quote, 900);

    in_contract(&env, || {
        storage::set_oracle_config(&env, &quote, &reflector_single(&reflector, &quote, 900));
        validate_oracle_config(&env, &asset, &config);
    });
}

/// The quote rule covers the anchor source too. Here the primary is a RedStone
/// feed that raises nothing, so only the anchor's quoted base can be what
/// reverted.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn anchor_with_an_unconfigured_quote_reverts_invalid_oracle_base() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let mut config =
        redstone_primary_reflector_anchor(&env, &feed, "PRIMARY", &reflector, &asset, 900);
    let OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(ref mut anchor)) =
        config.anchor
    else {
        panic!("fixture builds a Reflector anchor");
    };
    anchor.base = ReflectorBase::Quoted(quote);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// Source decimals are not bounded here. The `[MIN_ORACLE_DECIMALS,
/// MAX_ORACLE_DECIMALS]` bound lives in
/// `contracts/governance/src/validate/oracle_config.rs`, on the input the
/// governance resolver turns into an `AssetOracleConfig` — an owner calling the
/// aggregator directly is not held to it. Above 18 the two normalizers diverge
/// (`normalize_positive_price` downscales, `try_normalize_positive_price`
/// returns `None`), which is why the hard path carries a source-family backstop
/// for a leg the soft read rejected and the replay accepted.
#[test]
fn oracle_config_validation_does_not_bound_source_decimals() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let mut config = redstone_single(&env, &feed, "BTC/USD", 900);
    let OracleSourceConfig::RedStone(ref mut primary) = config.primary else {
        panic!("fixture builds a RedStone primary");
    };
    primary.decimals = 30;

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// TWAP record counts are not re-checked here. A zero-record window stores
/// cleanly and only reverts once something reads it, where
/// `validate_twap_records` runs in both read disciplines. The config-time bound
/// lives with the governance input validator.
#[test]
fn oracle_config_validation_does_not_recheck_twap_records() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let config = reflector_twap(&reflector, &asset, 0, 900);

    in_contract(&env, || validate_oracle_config(&env, &asset, &config));
}

/// A band can be walked, not teleported. The replacement below is well formed
/// on its own terms and contains no part of the stored band, so it is rejected
/// before the containment probe ever reads a price — which is the point: one
/// transient print must not be able to relocate the band wholesale.
#[test]
#[should_panic(expected = "Error(Contract, #224)")]
fn set_sanity_band_reverts_for_a_disjoint_band() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let asset = Address::generate(&env);
    let stored = redstone_single(&env, &feed, "BTC/USD", 900);

    in_contract(&env, || {
        storage::set_oracle_config(&env, &asset, &stored);
        set_sanity_band(&env, asset.clone(), WAD * 10, WAD * 11);
    });
}

/// `is_usd_rooted` accepts a quote whose own primary prices straight to USD,
/// across all three source families, and refuses one that is itself quoted.
/// This is the predicate `require_usd_rooted` and the soft read both branch on.
#[test]
fn is_usd_rooted_maps_every_primary_source_family() {
    let env = Env::default();
    let (feed, _feed_client) = register_redstone_feed(&env);
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);

    let redstone = redstone_single(&env, &feed, "BTC/USD", 900);
    let OracleSourceConfig::RedStone(ref inner) = redstone.primary else {
        panic!("fixture builds a RedStone primary");
    };
    let mut xoxno = redstone.clone();
    xoxno.primary = OracleSourceConfig::Xoxno(inner.clone());

    assert!(is_usd_rooted(&redstone));
    assert!(is_usd_rooted(&xoxno));
    assert!(is_usd_rooted(&reflector_single(&reflector, &asset, 900)));
    assert!(!is_usd_rooted(&reflector_quoted(
        &reflector, &asset, &quote, 900
    )));
}

/// The hard form of the same predicate reverts on the shape it refuses.
#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn require_usd_rooted_reverts_for_a_quoted_primary() {
    let env = Env::default();
    let reflector = Address::generate(&env);
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);

    require_usd_rooted(&env, &reflector_quoted(&reflector, &asset, &quote, 900));
}
