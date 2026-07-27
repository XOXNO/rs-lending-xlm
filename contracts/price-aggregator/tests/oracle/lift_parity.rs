//! Differential tests: a legacy config must price identically through the
//! composable engine and through the v1 path.
//!
//! This is the migration gate. The upgrade is deployed before any market is
//! migrated, so every live market is priced by `lift_legacy` for as long as
//! that takes. A divergence here is not a failed test — it is a silent
//! repricing of a live market, and the shapes below are the ones actually
//! configured on mainnet today.

use super::*;
use crate::test_support::{
    in_contract, redstone_dual, redstone_primary_reflector_anchor, redstone_single,
    reflector_quoted, reflector_single, reflector_twap, register_redstone_feed, PricedReflector,
    TwapReflector, TWAP_MEAN_WAD, TWAP_OLDER_AGE_SECS,
};
use common::constants::WAD;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, String};

const NOW: u64 = 1_000_000;
const MAX_STALE: u64 = 3_600;

fn at_now(env: &Env) {
    env.ledger().set_timestamp(NOW);
}

/// Prices `asset` both ways and asserts every field of the feed agrees.
///
/// Compares the whole `PriceFeedRaw`, not just the price: `asset_decimals`
/// drives seize amounts and protocol fees in liquidation, and `timestamp` is
/// what any freshness display rests on.
fn assert_same_price(env: &Env, asset: &Address) {
    let mut legacy_cache = ResolutionContext::new(env);
    let legacy = crate::price::resolve_usd_price(&mut legacy_cache, asset);

    let mut engine_cache = ResolutionContext::new(env);
    let lifted = resolve(&mut engine_cache, &PriceKey::Token(asset.clone()), 0);

    assert_eq!(
        lifted.price_wad, legacy.price_wad,
        "lifted price diverges from the v1 path"
    );
    assert_eq!(
        lifted.asset_decimals, legacy.asset_decimals,
        "decimals drive seize amounts; they must not shift under migration"
    );
    assert_eq!(
        lifted.timestamp, legacy.timestamp,
        "reported freshness diverges"
    );
}

#[test]
fn test_redstone_single_prices_identically() {
    // The Hub 2 RWA shape: one push-oracle NAV feed.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    client.set_price(&String::from_str(&env, "USST"), &WAD);

    in_contract(&env, || {
        let asset = Address::generate(&env);
        let config = redstone_single(&env, &adapter, "USST", MAX_STALE);
        crate::storage::set_oracle_config(&env, &asset, &config);
        assert_same_price(&env, &asset);
    });
}

#[test]
fn test_redstone_dual_prices_identically() {
    // Two multi-feed legs that agree: exercises the midpoint on the lifted path
    // against v1's `midpoint_if_in_band`.
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    client.set_price(&String::from_str(&env, "P"), &WAD);
    client.set_price(&String::from_str(&env, "A"), &(WAD + WAD / 100));

    in_contract(&env, || {
        let asset = Address::generate(&env);
        let config = redstone_dual(&env, &adapter, "P", "A", MAX_STALE, 10_500, 9_524);
        crate::storage::set_oracle_config(&env, &asset, &config);
        assert_same_price(&env, &asset);
    });
}

#[test]
fn test_reflector_spot_prices_identically() {
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(PricedReflector, ());

    in_contract(&env, || {
        let asset = Address::generate(&env);
        let config = reflector_single(&reflector, &asset, MAX_STALE);
        crate::storage::set_oracle_config(&env, &asset, &config);
        assert_same_price(&env, &asset);
    });
}

#[test]
fn test_reflector_twap_prices_identically() {
    // The Hub 1 shape. A TWAP observation dates itself to its oldest sample, so
    // this also pins that the lifted path inherits the same timestamp rule.
    let env = Env::default();
    env.ledger().set_timestamp(NOW);
    let reflector = env.register(TwapReflector, ());

    in_contract(&env, || {
        let asset = Address::generate(&env);
        let mut config = reflector_twap(&reflector, &asset, 2, MAX_STALE);
        // TwapReflector reports a mean of 2 WAD; widen the band to admit it.
        config.min_sanity_price_wad = TWAP_MEAN_WAD - WAD / 2;
        config.max_sanity_price_wad = TWAP_MEAN_WAD + WAD / 2;
        crate::storage::set_oracle_config(&env, &asset, &config);

        assert_same_price(&env, &asset);

        let mut cache = ResolutionContext::new(&env);
        let feed = resolve(&mut cache, &PriceKey::Token(asset), 0);
        assert_eq!(
            feed.timestamp,
            NOW - TWAP_OLDER_AGE_SECS,
            "a TWAP dates itself to its oldest sample on both paths"
        );
    });
}

#[test]
fn test_reflector_quoted_base_prices_identically_through_scaled() {
    // The shape that becomes `PriceSource::Scaled` under the lift, and the one
    // most likely to diverge: v1 reprices inside the provider through
    // `PriceStatus`, the engine resolves the quote as a first-class dependency.
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(PricedReflector, ());
    let (adapter, client) = register_redstone_feed(&env);
    client.set_price(&String::from_str(&env, "QUOTE"), &WAD);

    in_contract(&env, || {
        let quote = Address::generate(&env);
        let quote_config = redstone_single(&env, &adapter, "QUOTE", MAX_STALE);
        crate::storage::set_oracle_config(&env, &quote, &quote_config);

        let asset = Address::generate(&env);
        let config = reflector_quoted(&reflector, &asset, &quote, MAX_STALE);
        crate::storage::set_oracle_config(&env, &asset, &config);

        assert_same_price(&env, &asset);
    });
}

#[test]
fn test_mixed_provider_dual_prices_identically() {
    // RedStone primary against a Reflector anchor: the cross-provider dual that
    // v1's provider-distinctness rule was written for.
    let env = Env::default();
    at_now(&env);
    let reflector = env.register(PricedReflector, ());
    let (adapter, client) = register_redstone_feed(&env);
    client.set_price(&String::from_str(&env, "P"), &WAD);

    in_contract(&env, || {
        let asset = Address::generate(&env);
        let config =
            redstone_primary_reflector_anchor(&env, &adapter, "P", &reflector, &asset, MAX_STALE);
        crate::storage::set_oracle_config(&env, &asset, &config);
        assert_same_price(&env, &asset);
    });
}

/// Seeds a legacy config whose only feed is already past `MAX_STALE`.
fn stale_legacy_asset(env: &Env, adapter: &Address) -> Address {
    let asset = Address::generate(env);
    let config = redstone_single(env, adapter, "USST", MAX_STALE);
    crate::storage::set_oracle_config(env, &asset, &config);
    asset
}

fn publish_stale(client: &mock_redstone::MockRedStonePriceFeedClient, env: &Env) {
    let stale_ms = (NOW - MAX_STALE - 1) * 1_000;
    client.set_price_data(&String::from_str(env, "USST"), &WAD, &stale_ms, &stale_ms);
}

// Parity has to hold for rejections too, not only for prices: a market that
// halts on one path and prices on the other is the worst outcome of a partial
// migration. The pair below asserts both directions - `no_std` rules out
// catching the revert inside one test, so each path gets its own.

#[test]
#[should_panic]
fn test_a_stale_legacy_feed_is_rejected_by_the_v1_path() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish_stale(&client, &env);

    in_contract(&env, || {
        let asset = stale_legacy_asset(&env, &adapter);
        let mut cache = ResolutionContext::new(&env);
        let _ = crate::price::resolve_usd_price(&mut cache, &asset);
    });
}

#[test]
#[should_panic]
fn test_the_same_stale_legacy_feed_is_rejected_by_the_engine() {
    let env = Env::default();
    at_now(&env);
    let (adapter, client) = register_redstone_feed(&env);
    publish_stale(&client, &env);

    in_contract(&env, || {
        let asset = stale_legacy_asset(&env, &adapter);
        let mut cache = ResolutionContext::new(&env);
        let _ = resolve(&mut cache, &PriceKey::Token(asset), 0);
    });
}
